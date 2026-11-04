//! Host tests for blocking channel send/receive (#77).
//!
//! These drive the real syscall dispatcher with synthetic trap frames, so
//! they exercise the same block / wake / preempt path as the kernel.  Every
//! test takes `TEST_LOCK`, because the registry, scheduler and channel table
//! are global.

use crate::context::{TrapFrame, REG_A0, REG_A1, REG_A2, REG_A3, REG_A7};
use crate::ipc::kernel::{channel, CHANNEL_CAPACITY, MSG_MAX};
use crate::memory::task::{register_task, registry, BlockReason, TaskDescriptor, TaskMode, TaskState};
use crate::scheduler::{current_task_idx, populate_ready_queue, reset_for_test, TEST_LOCK};
use crate::syscall::{dispatch, EAGAIN, EFAULT, EINVAL, EPERM, FLAG_NONBLOCK, SYS_RECEIVE, SYS_SEND};

/// One simulated task: its descriptor fields plus host memory standing in
/// for its region and a heap frame standing in for its saved context.
struct Sim {
    mem: Box<[u8; 256]>,
    frame: *mut TrapFrame,
}

impl Sim {
    fn buf(&mut self) -> usize {
        self.mem.as_mut_ptr() as usize
    }
}

fn new_frame() -> *mut TrapFrame {
    Box::leak(Box::new(TrapFrame { regs: [0; 32], mepc: 0, mstatus: 0, mcause: 0, mtval: 0 }))
}

/// Register a U-mode task with priority `pri` and channel masks; its memory
/// region covers a fresh host buffer.
fn task(id: u8, pri: u8, sends: u32, receives: u32) -> Sim {
    let mut mem = Box::new([0u8; 256]);
    let start = mem.as_mut_ptr() as usize;
    let frame = new_frame();
    let mut d = TaskDescriptor::new(id, pri, start, 256, start, start + 256).with_channels(sends, receives);
    d.stack_ptr = frame as usize;
    register_task(d);
    Sim { mem, frame }
}

fn idle(id: u8) -> *mut TrapFrame {
    let frame = new_frame();
    let mut d = TaskDescriptor::new(id, 0, 0, 16, 0, 0).with_mode(TaskMode::Machine);
    d.stack_ptr = frame as usize;
    register_task(d);
    frame
}

/// Fresh registry, scheduler and empty channels.
fn setup() -> std::sync::MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_test();
    for ch in 0..crate::config::MAX_CHANNELS {
        while channel(ch).unwrap().try_receive().is_some() {}
    }
    guard
}

/// Issue syscall `n` from the task whose frame is `frame`, as the trap
/// handler would (mepc already advanced past the `ecall`).
fn syscall(frame: *mut TrapFrame, n: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> *mut TrapFrame {
    // SAFETY: `frame` is a live heap frame owned by the test.
    unsafe {
        let f = &mut *frame;
        f.regs[REG_A7] = n;
        f.regs[REG_A0] = a0;
        f.regs[REG_A1] = a1;
        f.regs[REG_A2] = a2;
        f.regs[REG_A3] = a3;
        f.mepc = 0x1004;
        dispatch(frame)
    }
}

fn a0(frame: *mut TrapFrame) -> usize {
    // SAFETY: live heap frame owned by the test.
    unsafe { (*frame).regs[REG_A0] }
}

fn state(slot: usize) -> (TaskState, BlockReason) {
    let t = registry()[slot].unwrap();
    (t.state, t.blocked_on)
}

#[test]
fn consumer_blocks_on_empty_and_preempts_producer_when_data_arrives() {
    let _g = setup();
    let mut producer = task(0, 1, 0b1, 0);
    let mut consumer = task(1, 3, 0, 0b1);
    let _idle = idle(2);
    populate_ready_queue();
    assert_eq!(current_task_idx(), 1, "highest priority (consumer) runs first");

    // Consumer receives on the empty channel: it blocks, the ecall is
    // rewound for restart, and the producer is dispatched.
    let buf = consumer.buf();
    let next = syscall(consumer.frame, SYS_RECEIVE, 0, buf, 16, 0);
    assert_eq!(next, producer.frame);
    assert_eq!(state(1), (TaskState::Blocked, BlockReason::Receive(0)));
    // SAFETY: live heap frame.
    assert_eq!(unsafe { (*consumer.frame).mepc }, 0x1000, "ecall restarts on wake");
    assert_eq!(current_task_idx(), 0);

    // Producer sends: the consumer wakes and, outranking the producer,
    // runs immediately (CA-PIP: no lower-priority task delays it).
    producer.mem[..5].copy_from_slice(b"hello");
    let p = producer.buf();
    let next = syscall(producer.frame, SYS_SEND, 0, p, 5, 0);
    assert_eq!(a0(producer.frame), 0);
    assert_eq!(next, consumer.frame, "woken consumer preempts the producer");
    assert_eq!(current_task_idx(), 1);
    assert_eq!(state(0).0, TaskState::Ready);

    // The restarted receive now succeeds and copies the message out.
    let next = syscall(consumer.frame, SYS_RECEIVE, 0, buf, 16, 0);
    assert_eq!(next, consumer.frame);
    assert_eq!(a0(consumer.frame), 5);
    assert_eq!(&consumer.mem[..5], b"hello");
}

#[test]
fn producer_blocks_on_full_and_resumes_when_consumer_reads() {
    let _g = setup();
    let mut producer = task(0, 3, 0b1, 0);
    let mut consumer = task(1, 1, 0, 0b1);
    let _idle = idle(2);
    populate_ready_queue();
    assert_eq!(current_task_idx(), 0);

    let p = producer.buf();
    for i in 0..CHANNEL_CAPACITY {
        producer.mem[0] = i as u8;
        let next = syscall(producer.frame, SYS_SEND, 0, p, 1, 0);
        assert_eq!(next, producer.frame, "send {i} fits");
    }

    // One more: the channel is full, so the producer blocks.
    let next = syscall(producer.frame, SYS_SEND, 0, p, 1, 0);
    assert_eq!(next, consumer.frame);
    assert_eq!(state(0), (TaskState::Blocked, BlockReason::Send(0)));

    // The consumer frees a slot: the producer wakes and preempts it.
    let c = consumer.buf();
    let next = syscall(consumer.frame, SYS_RECEIVE, 0, c, 16, 0);
    assert_eq!(consumer.mem[0], 0, "FIFO: first message out first");
    assert_eq!(next, producer.frame);
    assert_eq!(current_task_idx(), 0);

    // The restarted send succeeds.
    producer.mem[0] = 99;
    let next = syscall(producer.frame, SYS_SEND, 0, p, 1, 0);
    assert_eq!(next, producer.frame);
    assert_eq!(a0(producer.frame), 0);
}

#[test]
fn nonblocking_variants_return_eagain() {
    let _g = setup();
    let mut t = task(0, 1, 0b1, 0b1);
    let _idle = idle(1);
    populate_ready_queue();
    let b = t.buf();
    syscall(t.frame, SYS_RECEIVE, 0, b, 16, FLAG_NONBLOCK);
    assert_eq!(a0(t.frame), EAGAIN);
    assert_eq!(state(0).0, TaskState::Running);
    for _ in 0..CHANNEL_CAPACITY {
        syscall(t.frame, SYS_SEND, 0, b, 1, FLAG_NONBLOCK);
    }
    syscall(t.frame, SYS_SEND, 0, b, 1, FLAG_NONBLOCK);
    assert_eq!(a0(t.frame), EAGAIN);
}

#[test]
fn endpoint_capability_and_arguments_are_checked() {
    let _g = setup();
    let mut t = task(0, 1, 0b01, 0b10); // sends on 0, receives on 1
    let _idle = idle(1);
    populate_ready_queue();
    let b = t.buf();

    syscall(t.frame, SYS_SEND, 1, b, 1, 0);
    assert_eq!(a0(t.frame), EPERM, "no sender endpoint for channel 1");
    syscall(t.frame, SYS_RECEIVE, 0, b, 1, 0);
    assert_eq!(a0(t.frame), EPERM, "no receiver endpoint for channel 0");
    syscall(t.frame, SYS_SEND, 0, b, MSG_MAX + 1, 0);
    assert_eq!(a0(t.frame), EINVAL, "message too long");
    syscall(t.frame, SYS_SEND, 99, b, 1, 0);
    assert_eq!(a0(t.frame), EINVAL, "no such channel");
    syscall(t.frame, SYS_SEND, 0, 0x10, 4, 0);
    assert_eq!(a0(t.frame), EFAULT, "buffer outside the task's memory");
    syscall(t.frame, SYS_RECEIVE, 1, b + 250, 16, 0);
    assert_eq!(a0(t.frame), EFAULT, "receive buffer runs past the region");
}
