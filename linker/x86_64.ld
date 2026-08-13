ENTRY(_start)

SECTIONS
{
    . = 1M;
    __kernel_start = .;

    .multiboot ALIGN(8) :
    {
        KEEP(*(.multiboot))
    }

    .text.boot ALIGN(16) :
    {
        KEEP(*(.text.boot))
    }

    .text ALIGN(16) :
    {
        *(.text .text.*)
    }

    .rodata ALIGN(16) :
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4096) :
    {
        *(.data .data.*)
    }

    .bss ALIGN(16) :
    {
        *(.bss .bss.*)
        *(COMMON)
    }

    .boot_stack ALIGN(4096) :
    {
        *(.boot_stack)
    }

    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
    }
}
