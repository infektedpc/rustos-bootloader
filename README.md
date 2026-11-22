# rustos-bootloader

Builds a bootable RustOS disk image: validates a kernel ELF, then wraps
it with a UEFI (or BIOS) bootloader into a GPT (or MBR) disk image using
the [`bootloader`](https://crates.io/crates/bootloader) crate (0.11),
vendored under `vendor/bootloader` with a patched `uefi` crate — see
"Vendored uefi patch" below.

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

Boot it (needs QEMU + OVMF, neither installed in this sandbox).
Copy the VARS file first — QEMU writes to it on every boot:

```sh
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/OVMF_VARS.fd
qemu-system-x86_64 -machine q35,accel=tcg -m 512M \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,file=/tmp/OVMF_VARS.fd \
  -drive format=raw,file=disk.img -serial stdio -display none
```

## Boot flow (what OVMF executes)

The UEFI/GPT image's EFI System Partition holds exactly three files:

| ESP path | Contents |
|---|---|
| `efi/boot/bootx64.efi` | The `bootloader` crate's embedded UEFI bootloader — what OVMF executes first (the default UEFI boot entry). |
| `kernel-x86_64` | The RustOS kernel ELF, loaded by the bootloader. |
| `boot.json` | The serialised default `BootConfig`. The bootloader reads it when present and falls back to the same default when absent, so writing it changes nothing at boot — it makes the config explicit and inspectable. |

OVMF reads the GPT, finds the ESP, runs `efi/boot/bootx64.efi`,
which loads `kernel-x86_64` and jumps to its entry point
(`bootloader_api` 0.11 `entry_point!`, receiving `BootInfo`). The
kernel prints to VGA and to serial COM1, so with `-serial stdio`
you see it boot in the terminal. Verify the ESP contents with
`rustos-image inspect disk.img` (it lists the whole ESP tree).

## Vendored uefi patch (the boot-loop fix)

**Problem.** On OVMF/QEMU the boot looped: the bootloader printed
"exiting boot services" and the machine reset, over and over, until
one cycle happened to get through and the kernel booted. Root cause,
read in the upstream source (`uefi` 0.38.0, `src/boot.rs`,
`exit_boot_services`): exiting boot services fails when the memory-map
key goes stale between the GetMemoryMap and ExitBootServices calls
(firmware-side map changes), the crate retries only **twice**, and
then deliberately cold-resets the machine. A transient race became a
reboot loop. The bootloader's own panic handler halts, so the reset
was not a panic — it was this deliberate reset path.

**Fix.** `vendor/uefi` is a verbatim copy of the `uefi` 0.38.0 crate
with exactly one behavioural change, in `src/boot.rs`: retry the
exit sequence up to 100 times (`MAX_EXIT_BOOT_SERVICES_ATTEMPTS`),
logging each failure (`ExitBootServices attempt N/100 failed: ...`),
and only reset after the cap is exhausted — the same approach the
Linux kernel takes. The full diff against the pristine crate is
`diff -r <registry uefi-0.38.0> vendor/uefi` (plus `RUSTOS-PATCH.md`
in that directory).

**Why the `bootloader` crate is vendored too.** The `bootloader`
crate's build script builds its UEFI binary with a separate nested
`cargo install bootloader-x86_64-uefi` invocation, which a `[patch]`
in this repo's own `Cargo.toml` cannot reach. So `vendor/bootloader`
is unmodified upstream `bootloader` 0.11.17 (cloned from
`rust-osdev/bootloader`, tag `v0.11.17`; only `examples/`,
`.github/`, and lint config removed), with one addition: a
`[patch.crates-io] uefi = { path = "../uefi" }` section in its
workspace-root `Cargo.toml`, which the nested install does honour.
This repo depends on it via `bootloader = { path =
"vendor/bootloader" }`. The vendored workspace's `Cargo.lock` was
regenerated so the nested `--locked` install accepts the patch.
`vendor/bootloader/.cargo/config.toml` (`bindeps = true`) and this
repo's `.cargo/config.toml` exist because the vendored manifest's
dev-dependencies use `artifact = "bin"`, which cargo refuses to
parse without that nightly-only flag — nothing here builds those
dev-dependencies.

**How to tell the patch is in the binary.** The built
`bootx64.efi` contains the string `ExitBootServices attempt ` (the
upstream message `Error retrieving the memory map for exiting the
boot services` is gone), and its debug paths point at
`vendor/uefi/src/boot.rs`.

**Re-applying on upgrade.** When upgrading `uefi`, copy the new
crate source over `vendor/uefi` (keeping `RUSTOS-PATCH.md`), re-apply
the change in `src/boot.rs` (search for `RUSTOS PATCH`), update the
`[patch]` if the version changes, regenerate the vendored
`Cargo.lock` (`cargo update -p uefi` inside `vendor/bootloader`),
and rebuild. When upgrading `bootloader`, replace
`vendor/bootloader` with the new tag, re-add the `[patch]` section,
and regenerate its lockfile the same way.

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
