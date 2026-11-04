//! Boot sequence: registration, capability check, scheduler hand-off.
//!
//! Compiled only for the RISC-V target (see `main.rs`).

use crate::demo;
use ferret::context::{trap, MSTATUS_MPIE, MSTATUS_MPP_MACHINE};
use ferret::generated::demo_tasks::{TASK_H, TASK_L, TASK_M};
use ferret::memory::task::{register_task, TaskDescriptor};
use ferret::{capability, clint, memory, scheduler, uart};
use riscv_rt::entry;

const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Base priority of the kernel-owned idle task — below every OML task.
const IDLE_PRIORITY: u8 = 0;

// ---------------------------------------------------------------------------
// Compile-time overlap check for the three CA-PIP demo task memory regions.
// ---------------------------------------------------------------------------
const _: () = memory::assert_no_overlap::<
    0x8008_1000, 0x8008_2000,   // task L: 4 KB
    0x8008_2000, 0x8008_3000,   // task M: 4 KB
>();
const _: () = memory::assert_no_overlap::<
    0x8008_2000, 0x8008_3000,   // task M: 4 KB
    0x8008_3000, 0x8008_4000,   // task H: 4 KB
>();

// Statically allocated stacks.  Each Stack<N> carries a canary at its lowest
// address that the scheduler checks on every tick.
static mut STACK_TASK_L: memory::Stack<4096> = memory::Stack::new();
static mut STACK_TASK_M: memory::Stack<4096> = memory::Stack::new();
static mut STACK_TASK_H: memory::Stack<4096> = memory::Stack::new();
static mut STACK_IDLE: memory::Stack<1024> = memory::Stack::new();

/// Boot entry point — called by riscv-rt after BSS zero-init and `.data` copy.
///
/// Registers the demo tasks and the idle task, validates capabilities,
/// prepares each task's initial frame, arms the timer, and starts the
/// highest-priority task.  Never returns: the boot stack is abandoned once
/// the first task runs.
#[entry]
fn kernel_main() -> ! {
    uart::uart_puts("====================================\n");
    uart::uart_puts("  Ferret booting...\n");
    uart::uart_puts("====================================\n");
    uart::uart_puts(KERNEL_NAME);
    uart::uart_puts(" v");
    uart::uart_puts(KERNEL_VERSION);
    uart::uart_puts("\n");
    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    print_memory_map();

    // --- Task registration --------------------------------------------------
    // Interrupts are not yet enabled; this is the safe single-threaded boot
    // window for populating TASK_REGISTRY.  Registration order must match
    // the slot order of MAX_INHERITED_PRIORITIES (L, M, H); the idle task is
    // registered last and has no capabilities, so its MIP slot is 0.
    //
    // addr_of! gives the base address of each stack buffer without creating a
    // Rust reference to the mutable static.
    let stack_l = core::ptr::addr_of!(STACK_TASK_L) as usize;
    let stack_m = core::ptr::addr_of!(STACK_TASK_M) as usize;
    let stack_h = core::ptr::addr_of!(STACK_TASK_H) as usize;
    let stack_idle = core::ptr::addr_of!(STACK_IDLE) as usize;

    register_task(TASK_L.into_descriptor(0, stack_l, demo::task_l));
    register_task(TASK_M.into_descriptor(1, stack_m, demo::task_m));
    register_task(TASK_H.into_descriptor(2, stack_h, demo::task_h));
    register_task(
        TaskDescriptor::new(3, IDLE_PRIORITY, stack_idle, memory::Stack::<1024>::size(), 0, 0)
            .with_entry(demo::idle as usize),
    );

    uart::uart_puts("Tasks registered: ");
    uart::uart_print_usize(memory::task_count());
    uart::uart_puts("\n");

    // --- Capability conflict check ----------------------------------------
    // Halts permanently if two tasks claim the same exclusive peripheral.
    capability::check_capability_conflicts(memory::registry());
    uart::uart_puts("Capability check passed.\n");

    // --- PMP configuration (feature = "pmp") -------------------------------
    memory::configure_pmp();

    // --- Scheduler init -------------------------------------------------------
    // Write precomputed MIP constants, populate the ready queue, and build
    // each task's first trap frame.  Tasks run in M-mode for now (MPP = M);
    // MPIE makes interrupts come on with the first mret.
    scheduler::init();
    let gp: usize;
    // SAFETY: reading gp has no side effects.  Tasks are linked into the
    // kernel image, so they must run with the kernel's global pointer.
    unsafe { core::arch::asm!("mv {}, gp", out(reg) gp) };
    // SAFETY: boot path, before any task runs; every registered task has an
    // entry point and its own statically allocated stack.
    unsafe {
        scheduler::prepare_frames(
            demo::task_returned as usize,
            gp,
            MSTATUS_MPP_MACHINE | MSTATUS_MPIE,
        );
    }
    uart::uart_puts("Scheduler initialised.\n");

    print_task_registry();

    // --- Interrupt setup ----------------------------------------------------
    // Point mtvec at the trap entry stub.  Direct mode: bits[1:0] = 0 means
    // all traps dispatch to the same handler address.
    // SAFETY: __trap_entry is 4-byte aligned in the .trap section.
    unsafe {
        core::arch::asm!(
            "la t0, __trap_entry",
            "csrw mtvec, t0",
            out("t0") _,
        );
    }

    clint::start_ticks();

    // Enable the machine timer interrupt source (MTIE, mie[7]).  Global
    // interrupts (mstatus.MIE) stay off on the boot stack: they come on when
    // the first task's frame is restored, because its mstatus has MPIE set.
    // csrsi only accepts 5-bit immediates and MTIE is bit 7, so use csrs.
    // SAFETY: enabling an interrupt source has no effect until MIE is set.
    unsafe {
        core::arch::asm!(
            "li   t0, 0x80",
            "csrs mie, t0",
            out("t0") _,
        );
    }

    uart::uart_puts("====================================\n");
    uart::uart_puts("Interrupts enabled. Starting tasks.\n");
    uart::uart_puts("====================================\n");

    // SAFETY: mtvec is set, the first task's frame was built by
    // prepare_frames, and nothing on the boot stack is needed afterwards.
    unsafe { trap::start_first_task(scheduler::first_frame()) }
}

/// Print the contents of the task registry over UART.
fn print_task_registry() {
    let reg = memory::registry();
    uart::uart_puts("Task registry:\n");
    for task in reg.iter().flatten() {
        uart::uart_puts("  task ");
        uart::uart_print_usize(task.id as usize);
        uart::uart_puts("  base_pri=");
        uart::uart_print_usize(task.priority as usize);
        uart::uart_puts("  mip=");
        uart::uart_print_usize(task.max_inherited_priority as usize);
        uart::uart_puts("  eff_pri=");
        uart::uart_print_usize(task.effective_priority() as usize);
        uart::uart_puts("  excl_caps=");
        uart::uart_print_hex(task.exclusive_cap_mask as usize);
        uart::uart_puts("  req_caps=");
        uart::uart_print_hex(task.required_cap_mask as usize);
        uart::uart_puts("  mem=[");
        uart::uart_print_hex(task.memory_start);
        uart::uart_puts(", ");
        uart::uart_print_hex(task.memory_end);
        uart::uart_puts(")\n");
    }
}

/// Print the linker-symbol memory map over UART.
fn print_memory_map() {
    extern "C" {
        static _stext:       u8;
        static _sdata:       u8;
        static _edata:       u8;
        static _sbss:        u8;
        static _ebss:        u8;
        static _stack_start: u8;
    }

    // SAFETY: Only the *addresses* of linker symbols are read, never
    // the data they point to.  riscv-rt guarantees these are valid.
    let (stext, sdata, edata, sbss, ebss, stack) = unsafe {
        (
            &_stext       as *const u8 as usize,
            &_sdata       as *const u8 as usize,
            &_edata       as *const u8 as usize,
            &_sbss        as *const u8 as usize,
            &_ebss        as *const u8 as usize,
            &_stack_start as *const u8 as usize,
        )
    };

    uart::uart_puts("Memory map:\n");
    uart::uart_puts("  .text start : "); uart::uart_print_hex(stext);  uart::uart_puts("\n");
    uart::uart_puts("  .data       : "); uart::uart_print_hex(sdata);  uart::uart_puts(" .. ");
    uart::uart_print_hex(edata); uart::uart_puts(" ("); uart::uart_print_usize(edata - sdata); uart::uart_puts(" bytes)\n");
    uart::uart_puts("  .bss        : "); uart::uart_print_hex(sbss);   uart::uart_puts(" .. ");
    uart::uart_print_hex(ebss);  uart::uart_puts(" ("); uart::uart_print_usize(ebss - sbss);   uart::uart_puts(" bytes)\n");
    uart::uart_puts("  stack top   : "); uart::uart_print_hex(stack);  uart::uart_puts("\n");
}
