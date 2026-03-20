// FerretOS — kernel entry point (Sprint 0)
//
// #![no_std] tells the Rust compiler not to link the standard library.  In
// a bare-metal environment there is no OS to provide file I/O, heap
// allocation, threads, or any of the other things `std` wraps.  We only get
// `core`, which is the truly portable subset of the Rust standard library.
#![no_std]
//
// #![no_main] tells the compiler that we are not providing a conventional
// `fn main()` that the C runtime will call.  Instead, riscv-rt's assembly
// boot code will jump directly to our `#[entry]` function after setting up
// the stack and zeroing BSS.
#![no_main]

mod uart;
mod panic;

// The `riscv_rt::entry` attribute macro does two things:
//   1. It renames our function to `_start_rust` in the generated object file.
//   2. riscv-rt's assembly stub (`boot.S`) calls `_start_rust` after the
//      low-level machine-mode initialisation is complete (stack pointer set,
//      BSS zeroed, .data copied from flash to RAM).
//
// The function signature must be `fn() -> !` — it must never return, because
// there is nothing to return to.  If we accidentally returned, execution would
// fall into whatever happens to be in memory after our stack frame, which is
// undefined behaviour on bare metal.
use riscv_rt::entry;

/// Kernel version information — intentionally kept as compile-time constants
/// so they end up in .rodata (flash) and cost zero RAM.
const KERNEL_NAME: &str = "FerretOS";
const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Boot entry point.
///
/// This is the first Rust code that runs after riscv-rt initialises the
/// processor state.  In Sprint 0 we:
///   1. Print a boot banner over UART so we know the system came up.
///   2. Print a few diagnostic lines (version, target, memory layout).
///   3. Spin forever — no scheduler yet, nothing else to do.
///
/// Later sprints will replace the infinite loop with capability table setup,
/// task spawning, and the microkernel event loop.
#[entry]
fn kernel_main() -> ! {
    // --- Boot banner -------------------------------------------------------
    //
    // This is the very first output the user sees.  We want it to be clear
    // and unambiguous so that "did the kernel boot?" is answered immediately
    // by looking at the serial log.  The trailing newline is intentional —
    // some terminal emulators need it to flush the line buffer.
    uart::uart_puts("====================================\n");
    uart::uart_puts("  Ferret booting...\n");
    uart::uart_puts("====================================\n");

    // --- Kernel information ------------------------------------------------
    uart::uart_puts(KERNEL_NAME);
    uart::uart_puts(" v");
    uart::uart_puts(KERNEL_VERSION);
    uart::uart_puts(" — Sprint 0 (bare-metal bring-up)\n");

    uart::uart_puts("Target : riscv32imac-unknown-none-elf\n");
    uart::uart_puts("Machine: QEMU virt\n");

    // Print a few key linker-defined symbols so we can verify the linker
    // script worked and the kernel loaded at the right addresses.
    //
    // We only reference symbols that riscv-rt's link.x actually defines:
    //   _stext       — start of .text (riscv-rt PROVIDE)
    //   _sdata/_edata — bounds of .data (defined in link.x SECTIONS)
    //   _sbss/_ebss   — bounds of .bss  (defined in link.x SECTIONS)
    //   _stack_start  — top of stack    (riscv-rt PROVIDE)
    //
    // Note: _etext is NOT exported by riscv-rt 0.12's link.x, so we skip it.
    {
        extern "C" {
            static _stext:       u8;
            static _sdata:       u8;
            static _edata:       u8;
            static _sbss:        u8;
            static _ebss:        u8;
            static _stack_start: u8;
        }

        // SAFETY: We only read the *addresses* of these linker symbols, never
        // dereference them as data.  riscv-rt guarantees they are set correctly
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

        uart::uart_puts("  .text start : ");
        uart::uart_print_hex(stext);
        uart::uart_puts("\n");

        uart::uart_puts("  .data       : ");
        uart::uart_print_hex(sdata);
        uart::uart_puts(" .. ");
        uart::uart_print_hex(edata);
        uart::uart_puts(" (");
        uart::uart_print_usize(edata - sdata);
        uart::uart_puts(" bytes)\n");

        uart::uart_puts("  .bss        : ");
        uart::uart_print_hex(sbss);
        uart::uart_puts(" .. ");
        uart::uart_print_hex(ebss);
        uart::uart_puts(" (");
        uart::uart_print_usize(ebss - sbss);
        uart::uart_puts(" bytes)\n");

        uart::uart_puts("  stack top   : ");
        uart::uart_print_hex(stack);
        uart::uart_puts("\n");
    }

    uart::uart_puts("====================================\n");
    uart::uart_puts("Kernel idle — Sprint 1 coming soon.\n");
    uart::uart_puts("====================================\n");

    // Spin forever.  Without a scheduler there is nothing else to do.
    // The loop body uses `wfi` (Wait For Interrupt) to avoid burning CPU
    // cycles in QEMU unnecessarily.  We import it inline rather than adding a
    // top-level `use` so the dependency is obvious at the call site.
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
