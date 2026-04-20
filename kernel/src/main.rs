//! FerretOS kernel entry point.
#![no_std]
#![no_main]
#![feature(naked_functions)]

pub mod clint;
pub mod config;
pub mod context;
pub mod memory;
mod uart;
mod panic;

use memory::task::{TaskDescriptor, register_task};
use riscv_rt::entry;

const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Compile-time overlap check for the two demo task memory regions.
// If the ranges were ever edited to overlap this becomes a build error.
// ---------------------------------------------------------------------------
const _: () = memory::assert_no_overlap::<
    0x8008_1000, 0x8008_2000,   // task 0: 4 KB
    0x8008_2000, 0x8008_3000,   // task 1: 4 KB
>();

// Statically allocated stacks for the two demo tasks (Sprint 2 demo).
// These live in .bss and are zero-initialised before kernel_main runs.
static mut STACK_TASK0: memory::Stack<4096> = memory::Stack::new();
static mut STACK_TASK1: memory::Stack<4096> = memory::Stack::new();

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
    uart::uart_puts(" — Sprint 2 (memory + registry)\n");
    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    print_memory_map();

    // --- Task registration (Issues #22, #23) --------------------------------
    // Interrupts are not yet enabled; this is the safe single-threaded boot
    // window for populating TASK_REGISTRY.
    //
    // addr_of! gives the base address of each stack buffer without creating a
    // Rust reference to the mutable static (which is UB-prone under the Rust
    // 2024 static-mut-refs rules).
    // SAFETY: boot path, single-threaded, interrupts not yet enabled.
    let (stack0_base, stack1_base) = unsafe {
        (
            core::ptr::addr_of!(STACK_TASK0) as usize,
            core::ptr::addr_of!(STACK_TASK1) as usize,
        )
    };

    register_task(TaskDescriptor::new(
        0,              // id
        2,              // priority (higher = more important)
        stack0_base,
        4096,
        0x8008_1000,    // memory_start
        0x8008_2000,    // memory_end
    ));

    register_task(TaskDescriptor::new(
        1,              // id
        1,              // priority
        stack1_base,
        4096,
        0x8008_2000,    // memory_start
        0x8008_3000,    // memory_end
    ));

    let count = memory::task_count();
    uart::uart_puts("Tasks registered: ");
    uart::uart_print_usize(count);
    uart::uart_puts("\n");

    print_task_registry();

    // --- Interrupt setup (Issues #14, #15) ----------------------------------

    // Point mtvec at our trap entry stub.  Direct mode: bits[1:0] = 0 means
    // all traps dispatch to the same handler address.
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
        uart::uart_puts("  pri=");
        uart::uart_print_usize(task.priority as usize);
        uart::uart_puts("  stack_base=");
        uart::uart_print_hex(task.stack_base);
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
