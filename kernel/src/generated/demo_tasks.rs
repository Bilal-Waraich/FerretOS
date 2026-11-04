// This file has been generated from demo_tasks.oml

#[allow(dead_code)]

pub static TASK_L: TaskConfig = TaskConfig {
	priority: 1,
	stack_size: 4096,
	memory_start: 0x800B_8000,
	memory_end: 0x800B_9000,
	exclusive_cap_mask: 0x0000_0001,
	shared_cap_mask: 0x8000_0000,
	required_cap_mask: 0,
	sends_mask: 0,
	receives_mask: 0,
};

pub static TASK_M: TaskConfig = TaskConfig {
	priority: 2,
	stack_size: 4096,
	memory_start: 0x800B_9000,
	memory_end: 0x800B_A000,
	exclusive_cap_mask: 0,
	shared_cap_mask: 0x8000_0000,
	required_cap_mask: 0,
	sends_mask: 0,
	receives_mask: 0,
};

pub static TASK_H: TaskConfig = TaskConfig {
	priority: 3,
	stack_size: 4096,
	memory_start: 0x800B_A000,
	memory_end: 0x800B_B000,
	exclusive_cap_mask: 0,
	shared_cap_mask: 0x8000_0000,
	required_cap_mask: 0x0000_0001,
	sends_mask: 0,
	receives_mask: 0,
};
