//! FerretOS kernel entry point.
#![no_std]
#![no_main]

mod context;
mod uart;
mod panic;

use riscv_rt::entry;

const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Boot entry point — called by riscv-rt after BSS zero-init and `.data` copy.
///
/// Prints a boot banner with memory-map diagnostics, then waits for interrupts.
/// The `wfi` spin is temporary; Sprint 1 replaces it with the timer ISR loop.
#[entry]
fn kernel_main() -> ! {
    uart::uart_puts("====================================\n");
    uart::uart_puts("  Ferret booting...\n");
    uart::uart_puts("====================================\n");

    uart::uart_puts(KERNEL_NAME);
    uart::uart_puts(" v");
    uart::uart_puts(KERNEL_VERSION);
    uart::uart_puts(" — Sprint 0 (bare-metal bring-up)\n");

    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    {
        extern "C" {
            static _stext:       u8;
            static _sdata:       u8;
            static _edata:       u8;
            static _sbss:        u8;
            static _ebss:        u8;
            static _stack_start: u8;
        }

        // SAFETY: Only the *addresses* of linker symbols are read, never the
        // data they point to.  riscv-rt guarantees these symbols are valid
        // before our entry function is called.
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

    uart::uart_puts("====================================\n");
    uart::uart_puts("Kernel idle — Sprint 1 coming soon.\n");
    uart::uart_puts("====================================\n");

    loop {
        // SAFETY: `wfi` is a privileged hint instruction; safe in M-mode.
        unsafe { core::arch::asm!("wfi") };
    }
}
