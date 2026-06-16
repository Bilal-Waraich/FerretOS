// Auto-generated from tasks/task.oml by the OML transpiler.
// Do not edit manually — regenerate with: OML_BIN=/path/to/oml cargo build

#[allow(dead_code, clippy::upper_case_acronyms)]
pub enum Peripheral {
    UART0,
    UART1,
    GPIO0,
    GPIO1,
    SPI0,
    I2C0,
    NONE,
}

#[derive(Clone, Copy)]
pub struct TaskConfig {
    pub priority: u8,
    pub stack_size: u32,
    pub memory_start: u32,
    pub memory_end: u32,
    pub exclusive_cap_mask: u32,
    pub shared_cap_mask: u32,
    pub required_cap_mask: u32,
}
