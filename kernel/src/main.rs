//! FerretOS kernel entry point.
#![no_std]
#![no_main]
#![feature(naked_functions)]

pub mod clint;
pub mod context;
mod uart;
mod panic;

use riscv_rt::entry;

const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Boot entry point — called by riscv-rt after BSS zero-init and `.data` copy.
///
/// Initialises UART diagnostics, arms the CLINT timer, enables machine-mode
/// interrupts, then enters a counter loop that proves the interrupt path works
/// (Issue #17).
#[entry]
fn kernel_main() -> ! {
    uart::uart_puts("====================================\n");
    uart::uart_puts("  Ferret booting...\n");
    uart::uart_puts("====================================\n");
    uart::uart_puts(KERNEL_NAME);
    uart::uart_puts(" v");
    uart::uart_puts(KERNEL_VERSION);
    uart::uart_puts(" — Sprint 1 (interrupts + context)\n");
    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    print_memory_map();

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

    // --- Demo task (Issue #17) ----------------------------------------------
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
