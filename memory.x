/* RP2040 memory layout for RAK11300
 *
 * Flash : 2 MB external QSPI (W25Q16JV) mapped at 0x10000000 via XIP.
 *         The first 256 bytes are reserved for the second-stage bootloader
 *         (BOOT2), supplied by the rp2040-boot2 crate.
 *
 * RAM   : 256 KB of the RP2040's 264 KB on-chip SRAM.
 *         The remaining 8 KB (SRAM4 / SRAM5 scratch banks at 0x20040000)
 *         are left out of the main RAM region; cortex-m-rt does not need
 *         them and the RP2040 SDK uses them for inter-core mailboxes.
 */

MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2M - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}

/* Tell cortex-m-rt where to place the .boot2 section. */
EXTERN(BOOT2_FIRMWARE)

SECTIONS {
    .boot2 ORIGIN(BOOT2) :
    {
        KEEP(*(.boot2));
    } > BOOT2
} INSERT BEFORE .text;
