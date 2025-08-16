mod parsers;
mod statemap;
mod tracepoints;
mod types;

use crate::parsers::Action;
use crate::parsers::ClockData;
use crate::parsers::Event;
use crate::statemap::StatemapInputDatum;
use crate::statemap::StatemapInputState;
use crate::types::CpuState;
use byteorder::BigEndian;
use byteorder::LittleEndian;
use clap::Parser;
use compact_str::ToCompactString;
use compact_str::format_compact;
use eyre::Context;
use eyre::eyre;
use linux_perf_data::Endianness;
use linux_perf_data::PerfFileReader;
use linux_perf_data::PerfFileRecord;
use linux_perf_data::linux_perf_event_reader::RawData;
use linux_perf_data::linux_perf_event_reader::RecordType;
use linux_perf_data::linux_perf_event_reader::SampleRecord;
use std::collections::HashMap;
use std::io::Write;

mod cli {
    #[derive(clap_derive::Parser)]
    #[command(version, about)]
    /// Parse perf.data and generate statemeap
    pub struct Cli {
        #[clap(short, long)]
        pub verbose: bool,
        /// The name of the perf.data file to parse
        pub input: String,
        /// The name of the output file to write
        pub output: Option<String>,
    }
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = cli::Cli::parse();

    let file = std::fs::File::open(cli.input)?;
    let reader = std::io::BufReader::new(file);
    let PerfFileReader {
        mut perf_file,
        mut record_iter,
    } = PerfFileReader::parse_file(reader)?;

    // A mapping of current state of a given CPU. This is needed to restore state
    // after a IRQ exit or softirq exit. We also serialize straight from these
    // objects to the output stream.
    let num_cups = perf_file
        .nr_cpus()?
        .ok_or_else(|| eyre!("Failed to get number of CPUs"))?
        .nr_cpus_available as usize;
    let mut states = Vec::with_capacity(num_cups);
    for cpuid in 0..num_cups {
        states.push(StatemapInputDatum::<CpuState> {
            entity: format_compact!("{cpuid}"),
            ..Default::default()
        });
    }
    let mut prev_states = states.clone();

    let file: &mut dyn Write = match cli.output {
        Some(output) => &mut std::fs::File::create(output)?,
        None => &mut std::io::stdout().lock(),
    };
    let mut writer = std::io::BufWriter::new(file);

    // Write header metadata.
    write_header(&perf_file, &mut writer)?;

    // Create a lookup table from event attribute index to conversion action
    let action_map = action_mapping(&perf_file)?;

    let start_time = perf_file
        .sample_time_range()?
        .ok_or_else(|| eyre!("No sample time range found"))?
        .first_sample_time;

    let mut ctr = 0;
    while let Some(record) = record_iter.next_record(&mut perf_file)? {
        match record {
            PerfFileRecord::EventRecord { attr_index, record } => {
                match record.record_type {
                    // We don't care about these events (we are not doing stack traces)
                    RecordType::MMAP | RecordType::MMAP2 | RecordType::KSYMBOL => {}
                    RecordType::FORK | RecordType::EXIT | RecordType::COMM => {
                        // Process lifecycle events, we don't use these
                        // (currently) Instead we get data from tracepoints.
                    }
                    // This we need to handle
                    RecordType::SAMPLE => {
                        ctr += 1;
                        let action = action_map[attr_index];
                        if action == Action::Ignore {
                            continue; // Skip ignored actions
                        }
                        let common = record.common_data()?;
                        let endian = record.parse_info.endian;
                        let sample = match endian {
                            Endianness::LittleEndian => SampleRecord::parse::<LittleEndian>(
                                record.data,
                                record.misc,
                                &record.parse_info,
                            )?,
                            Endianness::BigEndian => SampleRecord::parse::<BigEndian>(
                                record.data,
                                record.misc,
                                &record.parse_info,
                            )?,
                        };
                        //let parsed = record.parse()?;
                        let action = action_map[attr_index];
                        let event = Event::parse(
                            action,
                            sample.raw.ok_or_else(|| eyre!("No raw data for trace?"))?,
                            endian,
                        )
                        .wrap_err_with(|| {
                            format!("Failed to parse: {sample:?}, action {action:?} (at {ctr})")
                        })?;
                        let cpu = common.cpu.expect("CPU should be present");
                        let time =
                            common.timestamp.expect("Timestamp should be present") - start_time;
                        //println!("Event: {event:?} on CPU {cpu} at time {time}");
                        match event {
                            Event::BeginThread { state, comm, pid } => {
                                states[cpu as usize].state = state;
                                states[cpu as usize].tag = Some(format_compact!("{comm}:{pid}"));
                            }
                            Event::BeginOther { state, tag } => {
                                prev_states[cpu as usize].clone_from(&states[cpu as usize]);
                                states[cpu as usize].state = state;
                                states[cpu as usize].tag = Some(tag);
                            }
                            Event::End => {
                                states[cpu as usize].clone_from(&prev_states[cpu as usize]);
                            }
                            Event::Migrate { from, to } => {
                                assert!(from != to, "Cannot migrate to the same CPU");
                                states[to as usize].time = time;
                                states[to as usize].state = states[from as usize].state;
                                states[to as usize].tag =
                                    std::mem::take(&mut states[from as usize].tag);
                                states[from as usize].time = time;
                                states[from as usize].state = CpuState::Idle;
                                // The statemap tool doesn't deal with None correctly.
                                states[from as usize].tag = Some("".to_compact_string());
                            }
                        }
                        states[cpu as usize].time = time;
                        // Write the current state to the output
                        serde_json::to_writer(&mut writer, &states[cpu as usize])?;
                        writeln!(writer)?;
                    }
                    RecordType::LOST | RecordType::LOST_SAMPLES => {
                        // Warn the user about lost samples
                        log::warn!(
                            "There are lost samples. Data is incomplete and may not be \
                             trustworthy!"
                        );
                    }
                    _ => {
                        log::warn!("Unhandled record type: {:?}", record.record_type);
                    }
                }
            }
            PerfFileRecord::UserRecord(_raw_user_record) => {
                // None of these appear to be useful right now, though
                // * PERF_TIME_CONV could possibly be useful to convert
                //   timestamps, but none of the values line up with wall time
                //   from what I can see.
            }
        }
    }

    Ok(())
}

/// Create a mapping from event attribute index to action to take when seeing
/// it. `perf sched` contains several events we don't use. Ignore those
/// explicitly so we get a warning on any new events showing up.
fn action_mapping(perf_file: &linux_perf_data::PerfFile) -> Result<Vec<Action>, eyre::Error> {
    let mut event_map = Vec::with_capacity(perf_file.event_attributes().len());
    for entry in perf_file.event_attributes() {
        let name = entry
            .name()
            .ok_or_else(|| eyre!("Failed to get event name"))?;
        //let ids = &entry.event_ids;
        let action = match name {
            "irq:irq_handler_entry" => Action::EnterIrq,
            "irq:irq_handler_exit" => Action::ExitIrq,
            "irq:softirq_entry" => Action::EnterSoftirq,
            "irq:softirq_exit" => Action::ExitSoftirq,
            "irq:tasklet_entry" => Action::EnterTasklet,
            "irq:tasklet_exit" => Action::ExitTasklet,
            "sched:sched_migrate_task" => Action::Migrate,
            "sched:sched_process_fork" => Action::Ignore,
            "sched:sched_stat_iowait" => Action::Ignore,
            "sched:sched_stat_runtime" => Action::Ignore,
            "sched:sched_stat_sleep" => Action::Ignore,
            "sched:sched_stat_wait" => Action::Ignore,
            "sched:sched_switch" => Action::Switch,
            "sched:sched_wakeup_new" => Action::Ignore,
            "sched:sched_wakeup" => Action::Ignore,
            "sched:sched_waking" => Action::Ignore,
            "dummy:u" => Action::Ignore,
            _ => {
                log::warn!("Unknown event name {name}, ignoring it");
                Action::Ignore
            }
        };
        event_map.push(action);
    }
    Ok(event_map)
}

/// Write header with metadata. This is the first entry in the output file.
fn write_header(
    perf_file: &linux_perf_data::PerfFile,
    writer: &mut impl Write,
) -> Result<(), eyre::Error> {
    let mut states = HashMap::new();
    states.insert(
        "Idle".to_compact_string(),
        StatemapInputState {
            color: Some("#e0e0e0".to_compact_string()),
            value: CpuState::Idle as usize,
        },
    );
    states.insert(
        "Irq".to_compact_string(),
        StatemapInputState {
            color: Some("#FF0000".to_compact_string()),
            value: CpuState::Irq as usize,
        },
    );
    states.insert(
        "Softirq".to_compact_string(),
        StatemapInputState {
            color: Some("#FF8000".to_compact_string()),
            value: CpuState::Softirq as usize,
        },
    );
    states.insert(
        "Tasklet".to_compact_string(),
        StatemapInputState {
            color: Some("#FFBF00".to_compact_string()),
            value: CpuState::Tasklet as usize,
        },
    );
    states.insert(
        "Kernel".to_compact_string(),
        StatemapInputState {
            color: Some("#2E4E00".to_compact_string()),
            value: CpuState::Kernel as usize,
        },
    );
    states.insert(
        "User".to_compact_string(),
        StatemapInputState {
            color: Some("#9BC362".to_compact_string()),
            value: CpuState::User as usize,
        },
    );
    // (Attempt to) compute time.
    let time_range = perf_file
        .sample_time_range()
        .wrap_err("Failed to get sample time range")?
        .ok_or_else(|| eyre!("No sample time range found"))?;
    let clock_data = perf_file.feature_section_data(linux_perf_data::Feature::CLOCK_DATA);
    let ts = match clock_data {
        Some(data) => {
            let parser = RawData::Single(data);
            let clock = ClockData::parse(parser, perf_file.endian())
                .wrap_err("Failed to parse CLOCK_DATA feature")?;
            // The first sample is not the same as the clock data sync point. I have seen it
            // be around half a second difference typically on my laptop. So compensate.
            clock.wall_clock_ns + (time_range.first_sample_time - clock.clockid_time_ns)
        }
        None => {
            log::warn!(
                "No CLOCK_DATA feature found, no idea when this trace was taken (consider using \
                 -k CLOCK_MONOTONIC_RAW when recording the trace)"
            );
            0
        }
    };
    const NS_PER_S: u64 = 1_000_000_000;
    let metadata = statemap::StatemapInputMetadata {
        start: vec![ts / NS_PER_S, ts % NS_PER_S],
        title: "CPU".to_compact_string(),
        host: perf_file
            .hostname()
            .unwrap_or_default()
            .map(|s| s.to_compact_string()),
        entityKind: Some("CPU".to_compact_string()),
        states, // This can be filled with actual states if needed
    };
    serde_json::to_writer(&mut *writer, &metadata)?;
    writeln!(writer)?;
    Ok(())
}
