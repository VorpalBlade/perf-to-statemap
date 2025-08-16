//! Tracepoints for scheduling events.

use super::parser::tracepoint_parser;
use compact_str::CompactString;

tracepoint_parser!(
    #[event_name("sched:sched_switch")]
    pub struct SchedSwitch {
        prev_comm: CompactString,
        prev_pid: i32,
        prev_state: i64,
        next_comm: CompactString,
        next_pid: i32,
        next_prio: i32,
    }
);

tracepoint_parser!(
    #[event_name("sched:sched_migrate_task")]
    pub struct SchedMigrateTask {
        comm: CompactString,
        pid: i32,
        prio: i32,
        orig_cpu: i32,
        dest_cpu: i32,
    }
);
