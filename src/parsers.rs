use crate::tracepoints::Tracepoint;
use crate::tracepoints::irq::IrqHandlerEntry;
use crate::tracepoints::irq::SoftirqEntry;
use crate::tracepoints::irq::TaskletEntry;
use crate::tracepoints::parser::FormatParser;
use crate::tracepoints::sched::SchedMigrateTask;
use crate::tracepoints::sched::SchedSwitch;
use crate::types::CpuState;
use byteorder::BigEndian;
use byteorder::ByteOrder;
use byteorder::LittleEndian;
use compact_str::CompactString;
use compact_str::format_compact;
use linux_perf_data::Endianness;
use linux_perf_data::linux_perf_event_reader::RawData;

/// Parser for `CLOCK_DATA` *file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockData {
    pub version: u32, /* version = 1 */
    pub clockid: u32,
    pub wall_clock_ns: u64,
    pub clockid_time_ns: u64,
}

impl ClockData {
    pub fn parse(data: RawData<'_>, endian: Endianness) -> Result<Self, std::io::Error> {
        match endian {
            Endianness::LittleEndian => Self::parse_impl::<LittleEndian>(data),
            Endianness::BigEndian => Self::parse_impl::<BigEndian>(data),
        }
    }

    pub fn parse_impl<O: ByteOrder>(mut data: RawData<'_>) -> Result<Self, std::io::Error> {
        let version = data.read_u32::<O>()?;
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported clock data version: {version}"),
            ));
        }
        let clockid = data.read_u32::<O>()?;
        let wall_clock_ns = data.read_u64::<O>()?;
        let clockid_time_ns = data.read_u64::<O>()?;

        Ok(Self {
            version,
            clockid,
            wall_clock_ns,
            clockid_time_ns,
        })
    }
}

/// Describes what parser to use for a given tracepoint sample record.
#[derive(Debug, Clone)]
pub enum Action {
    Ignore,
    Switch(FormatParser),
    Migrate(FormatParser),
    EnterIrq(FormatParser),
    ExitIrq(FormatParser),
    EnterSoftirq(FormatParser),
    ExitSoftirq(FormatParser),
    EnterTasklet(FormatParser),
    ExitTasklet(FormatParser),
}

/// A parsed tracepoint sample record turns into an `Event`.
#[derive(Debug, Clone)]
pub enum Event {
    BeginThread {
        state: CpuState,
        comm: CompactString,
        pid: i32,
    },
    BeginOther {
        state: CpuState,
        tag: CompactString,
    },
    End,
    Migrate {
        from: i32,
        to: i32,
    },
}

impl Event {
    pub fn parse(
        action: &Action,
        data: RawData<'_>,
        endian: Endianness,
    ) -> Result<Self, eyre::Error> {
        match endian {
            Endianness::LittleEndian => Self::parse_impl::<LittleEndian>(action, data),
            Endianness::BigEndian => Self::parse_impl::<BigEndian>(action, data),
        }
    }

    pub fn parse_impl<O: ByteOrder>(
        action: &Action,
        data: RawData<'_>,
    ) -> Result<Self, eyre::Error> {
        // We need to use dynamic parsers here, since the tracepoint format does change
        // between kernel versions.
        match action {
            Action::Ignore => unreachable!(),
            Action::Switch(parser) => {
                let parsed = SchedSwitch::parse::<O>(parser, &data)?;

                Ok(Self::BeginThread {
                    state: Self::classify(parsed.next_comm.as_bytes()),
                    comm: parsed.next_comm,
                    pid: parsed.next_pid,
                })
            }
            Action::Migrate(parser) => {
                let parsed = SchedMigrateTask::parse::<O>(parser, &data)?;
                Ok(Self::Migrate {
                    from: parsed.orig_cpu,
                    to: parsed.dest_cpu,
                })
            }
            Action::EnterIrq(parser) => {
                let parsed = IrqHandlerEntry::parse::<O>(parser, &data)?;
                Ok(Self::BeginOther {
                    state: CpuState::Irq,
                    tag: format_compact!("IRQ {}: {}", parsed.irq, parsed.name),
                })
            }
            Action::ExitIrq(_parser) => Ok(Self::End),
            Action::EnterSoftirq(parser) => {
                let parsed = SoftirqEntry::parse::<O>(parser, &data)?;
                Ok(Self::BeginOther {
                    state: CpuState::Softirq,
                    tag: format_compact!("Softirq {}", parsed.vec),
                })
            }
            Action::ExitSoftirq(_parser) => Ok(Self::End),
            Action::EnterTasklet(parser) => {
                let parsed = TaskletEntry::parse::<O>(parser, &data)?;
                Ok(Self::BeginOther {
                    state: CpuState::Tasklet,
                    tag: format_compact!("Tasklet {:#x}", parsed.tasklet),
                })
            }
            Action::ExitTasklet(_parser) => Ok(Self::End),
        }
    }

    /// Attempt to classify into user space vs kernel space threads.
    ///
    /// Not very accurate.
    fn classify(comm: &[u8]) -> CpuState {
        if comm.starts_with(b"swapper/") {
            return CpuState::Idle;
        }
        if comm.starts_with(b"migration/") {
            return CpuState::Idle;
        }
        if comm.starts_with(b"ksoftirqd/") {
            return CpuState::Softirq;
        }
        if comm.starts_with(b"irq/") {
            return CpuState::Irq;
        }
        if comm.starts_with(b"kworker/") || comm.starts_with(b"rcu_") {
            return CpuState::Kernel;
        }
        // TODO: We should look at /proc/<pid>/stat (9th field) and check if the flags
        // contains PF_KTHREAD? Or look if /proc/<pid>/exe is an unreadable symlink
        // (ENOENT). Sigh. This would also mean we have to run on the same host, rather
        // than being able to post-process the data on a different machine (which is
        // something I need for embedded Linux development.)
        //
        // Maybe there is a better way?
        CpuState::User
    }
}
