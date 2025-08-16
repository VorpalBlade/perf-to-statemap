use crate::types::CpuState;
use byteorder::BigEndian;
use byteorder::ByteOrder;
use byteorder::LittleEndian;
use compact_str::CompactString;
use compact_str::format_compact;
use eyre::Context;
use eyre::eyre;
use linux_perf_data::Endianness;
use linux_perf_data::linux_perf_event_reader::RawData;
use std::ffi::CStr;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Ignore,
    Switch,
    Migrate,
    EnterIrq,
    ExitIrq,
    EnterSoftirq,
    ExitSoftirq,
    EnterTasklet,
    ExitTasklet,
}

/// A parsed tracepoint sample record turns into an `Event`.
#[derive(Debug, Clone)]
pub enum Event {
    BeginThread {
        state: CpuState,
        comm: CompactString,
        pid: u32,
    },
    BeginOther {
        state: CpuState,
        tag: CompactString,
    },
    End,
    Migrate {
        from: u32,
        to: u32,
    },
}

/// Ugh, defined in a C header in the kernel.
const TASK_COMM_LEN: usize = 16;

impl Event {
    pub fn parse(
        action: Action,
        data: RawData<'_>,
        endian: Endianness,
    ) -> Result<Self, eyre::Error> {
        match endian {
            Endianness::LittleEndian => Self::parse_impl::<LittleEndian>(action, data),
            Endianness::BigEndian => Self::parse_impl::<BigEndian>(action, data),
        }
    }

    pub fn parse_impl<O: ByteOrder>(
        action: Action,
        mut data: RawData<'_>,
    ) -> Result<Self, eyre::Error> {
        // The comments with the format descriptions below are taken from
        // /sys/kernel/tracing/events/irq/softirq_entry/format etc and is the
        // format as exported by the kernel. We probably shouldn't hard code this but
        // parse it at runtime?
        //
        // While the Linux kernel is GPLv2 this is part of the user space ABI, and thus
        // doesn't affect the license of this code. (IANAL, but that is my
        // interpretation.)
        match action {
            Action::Ignore => unreachable!(),
            Action::Switch => {
                /*
                field:unsigned short common_type;    offset:0;    size:2;    signed:0;
                field:unsigned char common_flags;    offset:2;    size:1;    signed:0;
                field:unsigned char common_preempt_count;    offset:3;    size:1;    signed:0;
                field:int common_pid;    offset:4;    size:4;    signed:1;

                field:char prev_comm[16];    offset:8;    size:16;    signed:0;
                field:pid_t prev_pid;    offset:24;    size:4;    signed:1;
                field:int prev_prio;    offset:28;    size:4;    signed:1;
                field:long prev_state;    offset:32;    size:8;    signed:1;
                field:char next_comm[16];    offset:40;    size:16;    signed:0;
                field:pid_t next_pid;    offset:56;    size:4;    signed:1;
                field:int next_prio;    offset:60;    size:4;    signed:1;
                */
                Self::parse_common::<O>(&mut data)?;
                let mut prev_comm = [0u8; TASK_COMM_LEN];
                data.read_exact(&mut prev_comm)?;
                let _prev_pid = data.read_u32::<O>()?;
                let _prev_prio = data.read_i32::<O>()?;
                let _prev_state = data.read_u64::<O>()?;
                let mut next_comm = [0u8; TASK_COMM_LEN];
                data.read_exact(&mut next_comm)?;
                let next_pid = data.read_u32::<O>()?;
                let _next_prio = data.read_i32::<O>()?;

                // Convert the C-style string to a Rust string
                let next_comm = CStr::from_bytes_until_nul(&next_comm)?;
                let next_comm = next_comm.to_bytes();
                Ok(Self::BeginThread {
                    state: Self::classify(next_comm),
                    comm: CompactString::from_utf8_lossy(next_comm),
                    pid: next_pid,
                })
            }
            Action::Migrate => {
                /*
                field:unsigned short common_type;    offset:0;    size:2;    signed:0;
                field:unsigned char common_flags;    offset:2;    size:1;    signed:0;
                field:unsigned char common_preempt_count;    offset:3;    size:1;    signed:0;
                field:int common_pid;    offset:4;    size:4;    signed:1;

                field:__data_loc char[] comm;    offset:8;    size:4;    signed:0;
                field:pid_t pid;    offset:12;    size:4;    signed:1;
                field:int prio;    offset:16;    size:4;    signed:1;
                field:int orig_cpu;    offset:20;    size:4;    signed:1;
                field:int dest_cpu;    offset:24;    size:4;    signed:1;
                */
                Self::parse_common::<O>(&mut data)?;
                // This is apparently a string offset?
                let _comm = data.read_u32::<O>()?;
                let _pid = data.read_u32::<O>()?;
                let _prio = data.read_i32::<O>()?;
                let orig_cpu = data.read_u32::<O>()?;
                let dest_cpu = data.read_u32::<O>()?;
                Ok(Self::Migrate {
                    from: orig_cpu,
                    to: dest_cpu,
                })
            }
            Action::EnterIrq => {
                /*
                field:unsigned short common_type;    offset:0;    size:2;    signed:0;
                field:unsigned char common_flags;    offset:2;    size:1;    signed:0;
                field:unsigned char common_preempt_count;    offset:3;    size:1;    signed:0;
                field:int common_pid;    offset:4;    size:4;    signed:1;

                field:int irq;    offset:8;    size:4;    signed:1;
                field:__data_loc char[] name;    offset:12;    size:4;    signed:0;
                */
                // Parse the common fields, we don't care about them
                Self::parse_common::<O>(&mut data)?;
                let irq = data.read_i32::<O>()?;
                let name = trace_string::<O>(12, &mut data)
                    .wrap_err_with(|| format!("Failed while parsing IRQ {irq}"))?;
                Ok(Self::BeginOther {
                    state: CpuState::Irq,
                    tag: format_compact!("IRQ {irq}: {name}"),
                })
            }
            Action::ExitIrq => Ok(Self::End),
            Action::EnterSoftirq => {
                /*
                field:unsigned short common_type;    offset:0;    size:2;    signed:0;
                field:unsigned char common_flags;    offset:2;    size:1;    signed:0;
                field:unsigned char common_preempt_count;    offset:3;    size:1;    signed:0;
                field:int common_pid;    offset:4;    size:4;    signed:1;

                field:unsigned int vec;    offset:8;    size:4;    signed:0;
                */
                Self::parse_common::<O>(&mut data)?;
                let vec_nr = data.read_u32::<O>()?;
                Ok(Self::BeginOther {
                    state: CpuState::Softirq,
                    tag: format_compact!("Softirq {vec_nr}"),
                })
            }
            Action::ExitSoftirq => Ok(Self::End),
            Action::EnterTasklet => {
                /*
                field:unsigned short common_type;    offset:0;    size:2;    signed:0;
                field:unsigned char common_flags;    offset:2;    size:1;    signed:0;
                field:unsigned char common_preempt_count;    offset:3;    size:1;    signed:0;
                field:int common_pid;    offset:4;    size:4;    signed:1;

                field:void * tasklet;    offset:8;    size:8;    signed:0;
                field:void * func;    offset:16;    size:8;    signed:0;
                */
                Self::parse_common::<O>(&mut data)?;
                let tasklet = data.read_u64::<O>()?;
                let _func = data.read_u64::<O>()?;
                Ok(Self::BeginOther {
                    state: CpuState::Tasklet,
                    tag: format_compact!("Tasklet {tasklet:#x}"),
                })
            }
            Action::ExitTasklet => Ok(Self::End),
        }
    }

    /// Parse the common header for all trace events. We don't actually use any
    /// of the data, but we need to advance over it.
    fn parse_common<O: ByteOrder>(data: &mut RawData<'_>) -> Result<(), eyre::Error> {
        let _common_type = data.read_u16::<O>()?;
        let _common_flags = data.read_u8()?;
        let _common_preempt_count = data.read_u8()?;
        let _common_pid = data.read_i32::<O>()?;
        Ok(())
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

/// Read a string via a pointer from the data
/// We need to compensate for the offset of the field as the parser is consuming
/// the buffer.
fn trace_string<'data, O: ByteOrder>(
    field_offset: u32,
    data: &mut RawData<'data>,
) -> Result<CompactString, eyre::Error> {
    let name_ptr = data.read_u32::<O>()?;
    // Offset an additional 4 bytes for the pointer itself
    let name_offset = (name_ptr & 0xffff) - field_offset - 4;
    // The length is in the upper 16 bits of the pointer
    let name_len = name_ptr >> 16;
    let name: RawData<'_> = data
        .get(name_offset as usize..(name_offset + name_len) as usize)
        .ok_or_else(|| eyre!("Failed to get string field"))?;
    let name = name.as_slice();
    let name = name.as_ref();
    // The string contains a trailing NUL byte, get rid of it.
    let name =
        CStr::from_bytes_until_nul(name).wrap_err("Could not find nul byte while parsing field")?;
    // Convert to a Rust string
    Ok(CompactString::from_utf8_lossy(name.to_bytes()))
}
