//! Tracepoints for IRQ events.

use super::parser::tracepoint_parser;
use compact_str::CompactString;

tracepoint_parser!(
    #[event_name("irq:irq_handler_entry")]
    pub struct IrqHandlerEntry {
        irq: i32,
        name: CompactString,
    }
);

tracepoint_parser!(
    #[event_name("irq:irq_handler_exit")]
    pub struct IrqHandlerExit {
        irq: i32,
        ret: i32,
    }
);

tracepoint_parser!(
    #[event_name("irq:softirq_entry")]
    pub struct SoftirqEntry {
        vec: i32,
    }
);

tracepoint_parser!(
    #[event_name("irq:softirq_exit")]
    pub struct SoftirqExit {
        vec: i32,
    }
);

tracepoint_parser!(
    #[event_name("irq:tasklet_entry")]
    pub struct TaskletEntry {
        tasklet: u64,
        func: u64,
    }
);

tracepoint_parser!(
    #[event_name("irq:tasklet_exit")]
    pub struct TaskletExit {
        tasklet: u64,
        func: u64,
    }
);
