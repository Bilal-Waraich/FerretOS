// This file has been generated from task.oml

#[allow(dead_code)]

#[derive(Debug, Clone, PartialEq)]
pub enum Peripheral {
	UART0,
	UART1,
	GPIO0,
	GPIO1,
	SPI0,
	I2C0,
	NONE,
}

#[derive(Debug, Clone)]
pub struct TaskConfig {
	pub priority: u8,
	pub stack_size: u32,
	pub memory_start: u32,
	pub memory_end: u32,
	pub exclusive_cap_mask: u32,
	pub shared_cap_mask: u32,
	pub required_cap_mask: u32,
}
