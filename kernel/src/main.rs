//! FerretOS kernel entry point.
#![no_std]
#![no_main]
#![feature(naked_functions)]

pub mod capability;
pub mod clint;
pub mod config;
pub mod context;
pub mod generated;
pub mod ipc;
#[cfg(kani)]
pub mod kani_proofs;
pub mod memory;
pub mod scheduler;
mod uart;
#[cfg(not(kani))]
mod panic;

use generated::demo_tasks::{TASK_H, TASK_L, TASK_M};
use memory::task::register_task;
use riscv_rt::entry;

const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

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

// ---------------------------------------------------------------------------
// CA-PIP 3-task demo (Issue #40)
//
// Peripheral bitmasks:
//   UART0 = bit 0 (peripheral ID 0)
//
// Task L (priority 1): holds UART0 exclusively, does slow work
// Task M (priority 2): CPU-bound, no shared caps with L or H
// Task H (priority 3): requires UART0, prints timestamps
//
// Expected CA-PIP behaviour:
//   MIP(L) = max(H.priority) = 3  → L.effective_priority = 3
//   L is NOT preempted by M (eff_pri 3 > M.priority 2)
//   H runs immediately when L releases UART0
// ---------------------------------------------------------------------------

// Statically allocated stacks for the three demo tasks.
// Each Stack<4096> lives in .bss and is zero-initialised before kernel_main.
static mut STACK_TASK_L: memory::Stack<4096> = memory::Stack::new();
static mut STACK_TASK_M: memory::Stack<4096> = memory::Stack::new();
static mut STACK_TASK_H: memory::Stack<4096> = memory::Stack::new();

/// Boot entry point — called by riscv-rt after BSS zero-init and `.data` copy.
///
/// Initialises UART diagnostics, registers demo tasks, arms the CLINT timer,
/// enables machine-mode interrupts, then enters a counter loop that proves
/// the interrupt path works (Issue #17).
#[entry]
fn kernel_main() -> ! {
    uart::uart_puts("====================================\n");
    uart::uart_puts("  Ferret booting...\n");
    uart::uart_puts("====================================\n");
    uart::uart_puts(KERNEL_NAME);
    uart::uart_puts(" v");
    uart::uart_puts(KERNEL_VERSION);
    uart::uart_puts(" — Sprint 5 (OML integration)\n");
    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    print_memory_map();

    // --- Task registration (Issues #22, #23, #40) ---------------------------
    // Interrupts are not yet enabled; this is the safe single-threaded boot
    // window for populating TASK_REGISTRY.
    //
    // CA-PIP 3-task demo:
    //   L (id=0, pri=1): holds UART0 exclusively, does slow work
    //   M (id=1, pri=2): CPU-bound, no capability contention
    //   H (id=2, pri=3): requires UART0, prints timestamps
    //
    // addr_of! gives the base address of each stack buffer without creating a
    // Rust reference to the mutable static (UB-prone under Rust 2024).
    let stack_l_base = core::ptr::addr_of!(STACK_TASK_L) as usize;
    let stack_m_base = core::ptr::addr_of!(STACK_TASK_M) as usize;
    let stack_h_base = core::ptr::addr_of!(STACK_TASK_H) as usize;

    register_task(TASK_L.into_descriptor(0, stack_l_base));
    register_task(TASK_M.into_descriptor(1, stack_m_base));
    register_task(TASK_H.into_descriptor(2, stack_h_base));

    let count = memory::task_count();
    uart::uart_puts("Tasks registered: ");
    uart::uart_print_usize(count);
    uart::uart_puts("\n");

    // --- Capability conflict check (Issues #28, #29) -----------------------
    // Scan all registered tasks for exclusive capability conflicts before any
    // task runs.  Halts permanently if a conflict is found.
    capability::check_capability_conflicts(memory::registry());
    uart::uart_puts("Capability check passed.\n");

    // --- PMP configuration (feature = "pmp", closes #32) -------------------
    // Map each task's memory region to a RISC-V PMP TOR entry.
    // No-op when the pmp feature is not enabled.
    memory::configure_pmp();

    // --- Scheduler init (Issues #35, #36, #37, #38, #39) --------------------
    // Write precomputed MIP constants, populate ready queue.
    // Must run before interrupts are enabled.
    scheduler::init();
    uart::uart_puts("Scheduler initialised.\n");

    print_task_registry();

    // --- Interrupt setup (Issues #14, #15) ----------------------------------

    // Point mtvec at our trap entry stub.  Direct mode: bits[1:0] = 0 means
    // all traps dispatch to the same handler address.
    // Not compiled under Kani (x86_64 host) — RISC-V CSR instructions are
    // never executed by proof harnesses.
    #[cfg(not(kani))]
    unsafe {
        core::arch::asm!(
            "la t0, __trap_entry",
            "csrw mtvec, t0",
        );
    }

    // Arm the first timer tick before enabling interrupts so the first MTIP
    // fires at a predictable time rather than immediately.
    clint::schedule_tick(clint::TICK_CYCLES);

    // Enable the machine timer interrupt source (MTIE, mie[7]) then enable
    // global machine-mode interrupts (MIE, mstatus[3]).  Reversing this order
    // would open a brief window where global interrupts are on but MTIE is not
    // yet set — harmless here but a bad habit for future critical sections.
    // csrsi only accepts 5-bit immediates; MTIE (bit 7) and MIE (bit 3)
    // exceed that, so we use csrs with a register-held mask instead.
    #[cfg(not(kani))]
    unsafe {
        core::arch::asm!(
            "li   t0, 0x80",        // MTIE mask
            "csrs mie, t0",         // set MTIE (mie[7])
            "li   t0, 0x8",         // MIE mask
            "csrs mstatus, t0",     // set MIE  (mstatus[3])
            out("t0") _,
        );
    }

    uart::uart_puts("====================================\n");
    uart::uart_puts("Interrupts enabled. Running demo.\n");
    uart::uart_puts("====================================\n");

    // Increment a counter; every 100 000 iterations print the value.
    // Timer ISR fires every 1 ms and prints "[TICK N]".
    // Interleaved output proves that mret correctly resumes this loop.
    let mut counter: u32 = 0;
    loop {
        counter = counter.wrapping_add(1);
        if counter % 100_000 == 0 {
            uart::uart_puts("counter: ");
            uart::uart_print_usize(counter as usize);
            uart::uart_puts("\n");
        }
    }
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
        uart::uart_puts("  excl_caps=0x");
        uart::uart_print_hex(task.exclusive_cap_mask as usize);
        uart::uart_puts("  req_caps=0x");
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
