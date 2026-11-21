# rustos-bootloader

Builds a bootable RustOS disk image: validates a kernel ELF, then wraps
it with a UEFI (or BIOS) bootloader into a GPT (or MBR) disk image using
the [`bootloader`](https://crates.io/crates/bootloader) crate (0.11).

## What it is

- **Host-side tool** — runs on your development machine, produces disk
  images. It is *not* the code that runs at boot time; the code that runs
  at boot is the `bootloader` crate's embedded UEFI/BIOS stages.
- **Kernel-side vs host-side split** — in `bootloader` 0.11 the old
  single crate was split: `bootloader` builds images (this repo),
  `bootloader_api` provides the kernel entry point and `BootInfo`
  (`rustos-kernel`). See `docs/BOOT.md`.

## Build (needs nightly)

Building this crate compiles the `bootloader` crate, whose build script
builds its UEFI/BIOS stages with `cargo -Zbuild-std` — a nightly-only
flag. The nested cargo invocations inherit the outer toolchain, so the
outer cargo must be nightly too; building with stable fails inside the
`bootloader` build script. `rust-toolchain.toml` in this repo pins
nightly, so rustup selects it automatically here. Make sure nightly and
its pieces exist first:

```sh
../rustos-toolchain/target/release/rustos-toolchain check
rustup toolchain install nightly
rustup component add --toolchain nightly rust-src llvm-tools-preview
rustup target add --toolchain nightly x86_64-unknown-none
```

Then:

```sh
cargo build --release
```

## Usage

```sh
# UEFI/GPT image (default)
./target/release/rustos-bootloader \
  --kernel ../rustos-kernel/target/x86_64-unknown-none/debug/rustos-kernel \
  --out disk.img

# BIOS/MBR image
./target/release/rustos-bootloader --kernel <kernel> --out disk.img --bios

# Only the ESP FAT partition (inspect it, or PXE/TFTP boot)
./target/release/rustos-bootloader --kernel <kernel> --out esp.img --fat
```

Boot it (needs QEMU + OVMF, neither installed in this sandbox):

```sh
qemu-system-x86_64 -drive file=disk.img,format=raw \
  -bios /usr/share/OVMF/OVMF_CODE.fd -serial stdio
```

## Tests

```sh
cargo test
```

Tests cover ELF validation (valid header, bad magic, 32-bit, big-endian,
wrong machine, zero entry point), the UEFI ESP layout / boot-step model,
and building a real UEFI/GPT image from a dummy kernel and checking for
the `EFI PART` GPT header at LBA 1. The image test triggers the
`bootloader` crate's stage builds on first run, so it is slower than the
rest of the suite.

## Layout

- `src/lib.rs` — ELF validation + `build_disk_image` / `build_uefi_fat_partition`.
- `src/uefi.rs` — pure-Rust model of the ESP layout, the UEFI boot steps,
  and the bootloader→kernel handoff.
- `src/main.rs` — CLI front end.
- `docs/BOOT.md` — UEFI flow, memory map, and how the kernel is loaded.
