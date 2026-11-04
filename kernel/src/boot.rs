//! Boot sequence: registration, capability check, scheduler hand-off.
//!
//! Compiled only for the RISC-V target (see `main.rs`).

use crate::demo;
use ferret::context::{boot_trap_stack_top, trap};
use ferret::generated::demo_tasks::{TASK_H, TASK_L, TASK_M};
use ferret::memory::task::{register_task, TaskDescriptor, TaskMode};
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
    0x800B_8000, 0x800B_9000,   // task L: 4 KB
    0x800B_9000, 0x800B_A000,   // task M: 4 KB
>();
const _: () = memory::assert_no_overlap::<
    0x800B_9000, 0x800B_A000,   // task M: 4 KB
    0x800B_A000, 0x800B_B000,   // task H: 4 KB
>();

// Task stacks.  Each fills its task's 4 KB memory region in TASKRAM: the
// linker places `.task_ram.<slot>` sections in slot order (kernel/memory.x),
// and `check_region_placement` verifies at boot that this matches the OML
// declarations.  The sections are NOLOAD, so canaries are written at boot.
#[link_section = ".task_ram.0"]
static mut STACK_TASK_L: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.1"]
static mut STACK_TASK_M: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.2"]
static mut STACK_TASK_H: memory::Stack<4096> = memory::Stack::new();
// Region for the fault-demo rogue task: TASKRAM slot 3.
#[cfg(feature = "fault-demo")]
#[link_section = ".task_ram.3"]
static mut STACK_ROGUE: memory::Stack<4096> = memory::Stack::new();

// The idle task runs in M-mode and is not PMP-constrained; its stack stays
// in kernel RAM.
static mut STACK_IDLE: memory::Stack<1024> = memory::Stack::new();

/// Halt unless every task's stack lies inside its declared memory region.
///
/// The OML region addresses and the linker's placement of `.task_ram.*` are
/// two descriptions of the same layout; PMP isolation is only meaningful if
/// they agree.  Boot-time fatal on mismatch (CLAUDE.md §6.5).
fn check_region_placement() {
    for task in memory::registry().iter().flatten() {
        if task.mode != TaskMode::User {
            continue;
        }
        let inside = task.stack_base >= task.memory_start
            && task.stack_base + task.stack_size <= task.memory_end;
        if !inside {
            uart::uart_puts("MEMORY LAYOUT ERROR: task ");
            uart::uart_print_usize(task.id as usize);
            uart::uart_puts(" stack at ");
            uart::uart_print_hex(task.stack_base);
            uart::uart_puts(" is outside its OML region [");
            uart::uart_print_hex(task.memory_start);
            uart::uart_puts(", ");
            uart::uart_print_hex(task.memory_end);
            uart::uart_puts(")\nSystem halted. Fix tasks/demo_tasks.oml or kernel/memory.x.\n");
            ferret::halt();
        }
    }
}

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
    // The fault-demo rogue task is not part of the OML task set, so it gets
    // no MIP slot of its own and runs at its base priority.
    #[cfg(feature = "fault-demo")]
    {
        let stack_rogue = core::ptr::addr_of!(STACK_ROGUE) as usize;
        // SAFETY: boot path; base of an unused Stack<4096> in a NOLOAD section.
        unsafe { memory::stack::write_canary(stack_rogue) };
        register_task(
            TaskDescriptor::with_capabilities(
                3, 1, stack_rogue, 4096, 0x800B_B000, 0x800B_C000,
                0, capability::CONSOLE_CAP_BIT, 0,
            )
            .with_entry(demo::task_rogue as usize),
        );
    }
    register_task(
        TaskDescriptor::new(memory::task_count() as u8, IDLE_PRIORITY, stack_idle, memory::Stack::<1024>::size(), 0, 0)
            .with_entry(demo::idle as usize)
            .with_mode(TaskMode::Machine),
    );

    // SAFETY: boot path, no task running; each address is the base of a
    // Stack<N> in a NOLOAD section that no one has used yet.
    unsafe {
        memory::stack::write_canary(stack_l);
        memory::stack::write_canary(stack_m);
        memory::stack::write_canary(stack_h);
    }
    check_region_placement();

    uart::uart_puts("Tasks registered: ");
    uart::uart_print_usize(memory::task_count());
    uart::uart_puts("\n");

    // --- Capability conflict check ----------------------------------------
    // Halts permanently if two tasks claim the same exclusive peripheral.
    capability::check_capability_conflicts(memory::registry());
    uart::uart_puts("Capability check passed.\n");

    // --- Channel endpoint check ----------------------------------------------
    // Halts if two tasks claim the same sender or receiver endpoint.
    capability::check_channel_conflicts(memory::registry());
    uart::uart_puts("Channel check passed.\n");

    // --- PMP configuration ------------------------------------------------
    // feature = "pmp": flash R+X, and the running task's region R+W
    // (reprogrammed on every switch).  Otherwise a permissive grant so U-mode
    // can execute at all.
    memory::configure_pmp();

    // --- Scheduler init -------------------------------------------------------
    // Write precomputed MIP constants, populate the ready queue, and build
    // each task's first trap frame on its kernel stack.  OML tasks run in
    // U-mode; the idle task runs in M-mode.  MPIE makes interrupts come on
    // with the first mret.
    scheduler::init();
    let gp: usize;
    // SAFETY: reading gp has no side effects.  Tasks are linked into the
    // kernel image, so they must run with the kernel's global pointer.
    unsafe { core::arch::asm!("mv {}, gp", out(reg) gp) };
    // SAFETY: boot path, before any task runs; every registered task has an
    // entry point and its own statically allocated stack.
    unsafe {
        scheduler::prepare_frames(demo::task_returned as usize, gp);
    }
    uart::uart_puts("Scheduler initialised.\n");

    print_task_registry();

    // --- Interrupt setup ----------------------------------------------------
    // Point mtvec at the trap entry stub.  Direct mode: bits[1:0] = 0 means
    // all traps dispatch to the same handler address.
    // mscratch must hold a kernel stack top before the first trap: the
    // entry stub swaps to it unconditionally.  Until the first task runs,
    // that is the dedicated boot trap stack.
    // SAFETY: __trap_entry is 4-byte aligned in the .trap section, and the
    // boot trap stack is a static reserved for exactly this purpose.
    unsafe {
        core::arch::asm!(
            "csrw mscratch, {stack}",
            "la t0, __trap_entry",
            "csrw mtvec, t0",
            stack = in(reg) boot_trap_stack_top(),
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
