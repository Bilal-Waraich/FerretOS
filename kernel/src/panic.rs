//! panic.rs — Kernel panic handler.
//!
//! # Why do we need a custom panic handler?
//!
//! In a `no_std` crate, Rust does not link in the standard library's panic
//! machinery (stack unwinding, formatted error output, process exit).  We
//! must supply a `#[panic_handler]` ourselves, or the linker will refuse to
//! build.
//!
//! # Design choices
//!
//! - We use `core::fmt::Write` and a tiny `UartWriter` struct so we can use
//!   the `write!` macro for convenient formatted output — no allocator needed.
//! - After printing the panic message we spin forever.  There is no OS to
//!   return to, and attempting to continue after a panic would be UB in many
//!   situations.  In a production kernel we might also trigger a hardware
//!   watchdog reset here.
//! - We print the source file and line number from `PanicInfo` so that
//!   reading a serial log is enough to locate the failing assertion without
//!   needing a debugger attached.

use core::fmt;
use core::fmt::Write;
use core::panic::PanicInfo;

use crate::uart;

/// A zero-sized struct that implements `core::fmt::Write` by forwarding
/// every character to the UART.
///
/// Having this adapter means we can use `write!(UartWriter, "…", …)` and get
/// the full power of Rust's formatting machinery (padding, hex, decimal, etc.)
/// without allocating a String.
pub struct UartWriter;

impl fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        uart::uart_puts(s);
        // We always succeed — if the UART is broken we have bigger problems,
        // and returning an error here would just trigger another panic.
        Ok(())
    }
}

/// The kernel panic handler.
///
/// Rust calls this function when any code in the kernel hits a failed
/// `assert!`, `unwrap()` on `None`/`Err`, out-of-bounds slice index, or an
/// explicit `panic!("…")` call.
///
/// The `-> !` return type means this function must never return.  We satisfy
/// that contract with the infinite loop at the end.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Print a clear separator so the panic message stands out in a noisy log.
    uart::uart_puts("\n\n!!! KERNEL PANIC !!!\n");
    uart::uart_puts("PANIC: ");

    // Try to extract and print the human-readable panic message.
    // `info.message()` returns an `Arguments` (the thing `format_args!` makes).
    // We can write it directly to our UartWriter with the `write!` macro.
    let mut writer = UartWriter;
    let _ = write!(writer, "{}", info.message());

    uart::uart_puts("\n");

    // Print the source location (file + line number) if it is available.
    // Panics triggered by the compiler (e.g. array out-of-bounds) always have
    // a location; explicit `panic!` with a literal does too.
    if let Some(location) = info.location() {
        uart::uart_puts("  --> ");
        uart::uart_puts(location.file());
        uart::uart_puts(":");
        uart::uart_print_usize(location.line() as usize);
        uart::uart_puts("\n");
    }

    uart::uart_puts("System halted.\n");

    // Spin forever.  We deliberately avoid any further function calls here
    // because stack corruption (a common panic cause) could make them unsafe.
    loop {
        // The `riscv::asm::wfi()` instruction would be ideal here to reduce
        // power draw, but we keep this simple and dependency-free for now.
        core::hint::spin_loop();
    }
}
