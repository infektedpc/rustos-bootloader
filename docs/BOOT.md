# BOOT.md — how RustOS boots

## Big picture

```
power on
  → UEFI firmware (OVMF in QEMU) or BIOS (SeaBIOS)
    → bootloader stage (BOOTX64.EFI on UEFI / boot sector + stages on BIOS)
      → kernel ELF (`kernel-x86_64`)
        → kernel `_start` (via `bootloader_api::entry_point!`)
```

The bootloader in this repo is **not** hand-written firmware code. It is
a host-side tool built on the [`bootloader`](https://crates.io/crates/bootloader)
crate (0.11), which embeds pre-built UEFI and BIOS bootloader stages and
assembles them with your kernel into a bootable disk image. The kernel-side
half of the same project — the entry point and the `BootInfo` struct — lives
in the separate [`bootloader_api`](https://crates.io/crates/bootloader_api)
crate (0.11), which `rustos-kernel` depends on.

> **Version note — the 0.9/0.10 split.** Older tutorials (and the first
> edition of *Writing an OS in Rust*) used a single `bootloader` crate that
> the kernel depended on directly (`bootloader::entry_point!`,
> `bootloader::BootInfo`). In 0.11 that crate became host-side only:
> `bootloader::DiskImageBuilder` builds images, while `bootloader_api`
> provides `entry_point!` and `BootInfo` to the kernel. Code written
> against the old single-crate API will not compile against 0.11.

## UEFI flow (default: `--uefi`)

1. **GPT.** `build_disk_image(uefi = true)` writes a GPT disk. The first
   usable partition is the EFI System Partition (ESP), a FAT filesystem.
2. **ESP contents.** The FAT partition holds:
   - `efi/boot/bootx64.efi` — the UEFI bootloader (the fallback path the
     firmware tries when no explicit boot entry exists);
   - `kernel-x86_64` — the kernel ELF;
   - `boot.json` — boot configuration (framebuffer, log level), written
     when a `BootConfig` is supplied.
3. **Firmware handoff.** OVMF finds the ESP, loads `bootx64.efi`, and calls
   its UEFI entry point with the UEFI system table.
4. **Kernel load.** The bootloader opens the FAT volume through UEFI's
   file protocol, reads `kernel-x86_64`, parses its ELF program headers,
   and maps its loadable segments into memory.
5. **Memory map.** The bootloader asks UEFI for the memory map
   (`GetMemoryMap`), then calls `ExitBootServices`. After that point no
   UEFI boot-service calls are possible; only the small runtime-services
   set remains, and this bootloader does not use it.
6. **Kernel entry.** The bootloader builds a `bootloader_api::BootInfo`
   (memory regions, physical-memory offset, framebuffer, kernel address)
   and jumps to the kernel entry point. The kernel's `entry_point!`
   macro generates the `_start` wrapper that receives it.

## BIOS flow (`--bios`)

The MBR disk carries a boot sector plus stages 2–4. The BIOS loads the
first sector, the stages chain-load each other, and the final stage does
the same job as the UEFI stage: read the kernel from the FAT partition,
build `BootInfo`, jump to the entry point. BIOS booting exists for
old hardware and for QEMU without OVMF; UEFI is the path to prefer.

## Memory map (what the kernel is told)

The bootloader translates the firmware memory map into
`bootloader_api::MemoryRegion`s, each with a `start`, an `end`
(exclusive), and a `kind`:

| Kind | Meaning |
| --- | --- |
| `Usable` | Free RAM the kernel may use for frames / heap. |
| `Bootloader` | Memory the bootloader itself occupies (page tables, `BootInfo`, loaded kernel). Do not reuse. |
| `UnknownUefi(u32)` / `UnknownBios(u32)` | Firmware-reported regions with a type the bootloader does not recognise. Do not use. |

The kernel (`rustos-kernel/src/memory.rs`) turns the `Usable` regions
into a frame allocator. The bootloader also optionally maps **all of
physical memory** at a virtual offset (`physical_memory_offset`) so the
kernel can walk page tables; the kernel requests that mapping in its
`BootloaderConfig` (`mappings.physical_memory = Some(Mapping::Dynamic)`).
Without it, `BootInfo.physical_memory_offset` is `None` and the kernel
cannot build its page-table walker — this is the single most common
"my kernel triple-faults on the first memory access" mistake.

## How the kernel is loaded

- The kernel ELF must be 64-bit, little-endian, `EM_X86_64`, with a
  non-zero `e_entry`. `validate_kernel_elf` checks exactly that before
  any image is built; see `src/lib.rs`.
- The kernel is **not** a flat binary. The bootloader parses ELF program
  headers and maps each `PT_LOAD` segment at its `p_vaddr`. The kernel
  must therefore be linked as a position-independent executable or for
  the address the bootloader maps it at (the default `BootloaderConfig`
  uses dynamic mapping, so a PIE kernel is the safe choice — the
  `rustos-kernel` link configuration does this).
- The kernel's `.bootloader-config` section (emitted by
  `bootloader_api::entry_point!`) tells the bootloader the kernel stack
  size (default 80 KiB) and which mappings to set up. If the kernel and
  the bootloader disagree on the `bootloader_api` version, the
  bootloader refuses to boot it.

## Building an image

```sh
# 1. Build the kernel (from rustos-kernel; needs nightly)
cargo build --target x86_64-unknown-none

# 2. Wrap it in a bootable image (from this repo)
cargo run --release -- \
  --kernel ../rustos-kernel/target/x86_64-unknown-none/debug/rustos-kernel \
  --out disk.img

# 3. Boot it (QEMU + OVMF; QEMU is not installed in this sandbox)
qemu-system-x86_64 -drive file=disk.img,format=raw \
  -bios /usr/share/OVMF/OVMF_CODE.fd -serial stdio
```

`--bios` instead of the default `--uefi` produces an MBR image bootable
under SeaBIOS (`qemu-system-x86_64 -drive file=disk.img,format=raw`).
`--fat` writes only the ESP FAT partition, for inspection or PXE/TFTP.

## What is not done here

- **No hand-written UEFI application.** The UEFI stage is the
  `bootloader` crate's embedded binary; this repo models its behaviour
  (`src/uefi.rs`) and drives its image builder, it does not reimplement
  UEFI in Rust. Writing a from-scratch UEFI app would mean depending on
  the `uefi` crate and the `x86_64-unknown-uefi` target directly — a
  possible follow-up, not needed to boot.
- **No secure boot.** The image is unsigned; OVMF must run with secure
  boot off (the default).
- **No live boot test in this sandbox.** QEMU is not installed here, so
  the image is verified structurally (GPT header, FAT contents) by the
  test suite, not by actually booting it. Boot it on a machine with QEMU
  + OVMF to see the kernel banner.
