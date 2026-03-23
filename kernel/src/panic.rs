//! Kernel panic handler — prints diagnostic to UART and halts.

use core::fmt;
use core::fmt::Write;
use core::panic::PanicInfo;

use crate::uart;

/// Zero-sized `fmt::Write` adapter that forwards every character to the UART.
///
/// Allows `write!(UartWriter, …)` without an allocator.
pub struct UartWriter;

impl fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        uart::uart_puts(s);
        Ok(())
    }
}

/// Kernel panic handler.
///
/// Prints the panic message and source location to UART, then spins forever.
/// Spinning rather than returning is mandatory: there is no OS to unwind into,
/// and the stack may already be corrupt.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    uart::uart_puts("\n\n!!! KERNEL PANIC !!!\nPANIC: ");
    let _ = write!(UartWriter, "{}", info.message());
    uart::uart_puts("\n");
    if let Some(location) = info.location() {
        uart::uart_puts("  --> ");
        uart::uart_puts(location.file());
        uart::uart_puts(":");
        uart::uart_print_usize(location.line() as usize);
        uart::uart_puts("\n");
    }
    uart::uart_puts("System halted.\n");
    loop {
        core::hint::spin_loop();
    }
}
