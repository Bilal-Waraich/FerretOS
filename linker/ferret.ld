/*
 * linker/ferret.ld — Linker script for FerretOS on the QEMU virt board.
 *
 * What does a linker script do?
 * The linker's job is to take all the compiled object files and stick their
 * sections (.text, .data, .bss, …) into the right spots in the final binary.
 * Without this file, the linker has no idea where our FLASH and RAM live on
 * this particular chip, so it would produce an ELF that crashes at address 0.
 *
 * Memory map for QEMU virt (riscv32):
 *   FLASH — 0x80000000 … 0x8007FFFF  (512 KB, read+execute)
 *           QEMU maps DRAM here; we treat the first 512 KB as "flash" to
 *           simulate a real embedded layout where code is stored in NOR flash.
 *   RAM   — 0x80080000 … 0x800BFFFF  (256 KB, read+write+execute)
 *           Immediately after "flash" in the same DRAM region.  Stack grows
 *           downward from the top of RAM.
 *
 * riscv-rt expectations:
 *   riscv-rt's own linker fragment (memory.x) normally defines MEMORY and
 *   the section layout.  Because we supply a complete script here, riscv-rt
 *   will use ours instead.  We must still provide the symbols it expects:
 *   _stext, _etext, _sdata, _edata, _sbss, _ebss, _stack_start.
 */

ENTRY(_start)   /* _start is the reset vector provided by riscv-rt's boot.S */

MEMORY
{
    /* rx  = readable + executable (code lives here) */
    FLASH (rx)  : ORIGIN = 0x80000000, LENGTH = 512K

    /* rwx = readable + writable + executable (data, bss, stack live here)  */
    /* We mark it executable in case we ever need to patch trampolines in    */
    /* RAM, but normal data and stack do not need the x bit on real hardware. */
    RAM   (rwx) : ORIGIN = 0x80080000, LENGTH = 256K
}

/*
 * PROVIDE lets riscv-rt's crt0.S reference these symbols without causing a
 * duplicate-definition error if riscv-rt happens to define them too.
 */
PROVIDE(_max_hart_id = 0);          /* single-hart system */
PROVIDE(_hart_stack_size = 2K);     /* per-hart stack if using SMP; unused for now */
PROVIDE(_heap_size = 0);            /* no heap allocator in Sprint 0 */

SECTIONS
{
    /*
     * .text — executable code.
     * The KEEP directive prevents the linker from discarding the reset vector
     * even if nothing in our Rust code explicitly references it by name.
     */
    .text ORIGIN(FLASH) :
    {
        _stext = .;                     /* riscv-rt symbol: start of text */

        KEEP(*(.init))                  /* riscv-rt's crt0.S reset vector */
        KEEP(*(.init.rust))             /* riscv-rt's Rust-level init hook */
        KEEP(*(.trap))                  /* trap/interrupt vector table */
        KEEP(*(.trap.rust))

        *(.text .text.*)                /* all Rust function bodies */

        . = ALIGN(4);
        _etext = .;                     /* riscv-rt symbol: end of text */
    } > FLASH

    /*
     * .rodata — read-only data: string literals, static tables, etc.
     * These go in FLASH too, since they don't need to be written at runtime.
     */
    .rodata : ALIGN(4)
    {
        _srodata = .;
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)          /* small read-only data (RISC-V convention) */
        . = ALIGN(4);
        _erodata = .;
    } > FLASH

    /*
     * .data — mutable globals that have non-zero initial values.
     * The initial values are stored in FLASH (LMA = Load Memory Address) and
     * must be copied to RAM (VMA = Virtual Memory Address) before main() runs.
     * riscv-rt's crt0.S does this copy using the _sdata/_edata/_sidata symbols.
     */
    .data : ALIGN(4)
    {
        _sdata = .;                     /* riscv-rt symbol */
        *(.data .data.*)
        *(.sdata .sdata.*)              /* small data section (GP-relative) */
        . = ALIGN(4);
        _edata = .;                     /* riscv-rt symbol */
    } > RAM AT > FLASH                  /* AT > FLASH: initialisation image in FLASH */

    /* Where in FLASH the .data initialisation image starts */
    _sidata = LOADADDR(.data);

    /*
     * .bss — mutable globals initialised to zero.
     * No need to store zeroes in FLASH; crt0.S zeroes this region at boot.
     */
    .bss (NOLOAD) : ALIGN(4)
    {
        _sbss = .;                      /* riscv-rt symbol */
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(4);
        _ebss = .;                      /* riscv-rt symbol */
    } > RAM

    /*
     * Stack lives at the very top of RAM and grows downward.
     * _stack_start is the initial value of the stack pointer (sp).
     * We place it at the end of RAM so that stack overflows crash into
     * unmapped space rather than silently corrupting BSS or data.
     */
    . = ORIGIN(RAM) + LENGTH(RAM);
    _stack_start = .;                   /* riscv-rt symbol */

    /*
     * ASSERT guards — fail the link (at build time, not at runtime) if our
     * code sections have grown past their memory budget.  Catching this early
     * is much friendlier than a mysterious crash on the target.
     * Note: each ASSERT must be a single line for lld compatibility.
     */
    ASSERT(_etext <= ORIGIN(FLASH) + LENGTH(FLASH), "ERROR: .text + .rodata overflow FLASH (512 KB budget exceeded)")
    ASSERT(_ebss <= ORIGIN(RAM) + LENGTH(RAM), "ERROR: .data + .bss overflow RAM (256 KB budget exceeded)")

    /*
     * Discard everything the C runtime would normally need but we don't:
     * exception frames, constructor/destructor lists, and debug metadata that
     * the linker generates for hosted environments.
     */
    /DISCARD/ :
    {
        *(.eh_frame)
        *(.ARM.exidx)
        *(.ARM.extab)
    }
}
