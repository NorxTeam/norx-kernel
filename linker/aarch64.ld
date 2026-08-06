ENTRY(_start)

SECTIONS
{
    . = 0;
    __kernel_start = .;

    .image_header ALIGN(8) :
    {
        KEEP(*(.image_header))
    }

    . = 0x80000;
    .text.boot ALIGN(16) :
    {
        KEEP(*(.text.boot))
    }

    .text ALIGN(2048) :
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

    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note .note.*)
    }
}
