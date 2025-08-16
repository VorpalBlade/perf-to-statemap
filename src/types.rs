/// The various states we report in the state map.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde_repr::Serialize_repr)]
#[repr(u8)]
pub enum CpuState {
    #[default]
    Idle,
    Irq,
    Softirq,
    Tasklet,
    Kernel,
    User,
}
