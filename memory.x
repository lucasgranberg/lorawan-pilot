MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100
    STORAGE : ORIGIN = 0x10100000, LENGTH = 4K
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
/* STORAGE is only provided here for documentation of LoRaWAN storage area.  Offset rather than memory address is used in the code. */
__storage = ORIGIN(STORAGE);
