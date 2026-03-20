/*
 * kernel/memory.x — Memory region definitions for FerretOS on QEMU virt.
 *
 * How this works with riscv-rt 0.12:
 *   riscv-rt's link.x references memory regions by the names REGION_TEXT,
 *   REGION_RODATA, REGION_DATA, REGION_BSS, REGION_HEAP, and REGION_STACK.
 *   It does NOT define these regions itself — that is our job.  We define them
 *   here, and our build.rs adds this directory to the linker search path so
 *   that when link.x is processed it can resolve the REGION_* names.
 *
 * Memory map for QEMU virt (riscv32):
 *   QEMU places DRAM starting at 0x80000000.  We split it into two logical
 *   regions to simulate a typical embedded layout:
 *
 *   FLASH (0x80000000, 512 KB) — code and read-only data (execute-in-place).
 *   RAM   (0x80080000, 256 KB) — mutable data, BSS, heap placeholder, stack.
 *
 * We then alias each REGION_* name to the appropriate physical region.
 * riscv-rt places .text and .rodata into REGION_TEXT / REGION_RODATA (both
 * in FLASH), and .data / .bss / stack into their respective RAM regions.
 */

MEMORY
{
    /* rx = readable + executable */
    FLASH (rx)  : ORIGIN = 0x80000000, LENGTH = 512K

    /* rwx = readable + writable + executable */
    RAM   (rwx) : ORIGIN = 0x80080000, LENGTH = 256K
}

/*
 * Map the abstract REGION_* names that riscv-rt's link.x expects onto our
 * physical memory regions above.  REGION_HEAP and REGION_STACK share RAM
 * with data/BSS; riscv-rt's _heap_size = 0 default means REGION_HEAP is
 * fictitious (zero bytes), so no space is actually consumed for a heap.
 */
REGION_ALIAS("REGION_TEXT",   FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA",   RAM);
REGION_ALIAS("REGION_BSS",    RAM);
REGION_ALIAS("REGION_HEAP",   RAM);
REGION_ALIAS("REGION_STACK",  RAM);

/*
 * Discard exception-handling frame data.
 *
 * Even with `panic = "abort"` in the release profile, the compiler and
 * compiler-builtins can still emit .eh_frame sections.  On a bare-metal
 * RISC-V target these sections are useless (there is no runtime unwinder),
 * and the PCREL relocations they contain go out of range because the section
 * ends up far from the code it references.  Discarding them here eliminates
 * the link error and sheds a few hundred bytes from the binary.
 */
SECTIONS
{
    /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
}
