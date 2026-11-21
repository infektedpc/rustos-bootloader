//! `rustos-bootloader`: build a bootable RustOS disk image.
//!
//! Two jobs, kept separate:
//!
//! 1. **Validate the kernel ELF** ([`validate_kernel_elf`]) — the image
//!    builder refuses to wrap a file that is not a 64-bit little-endian
//!    x86-64 ELF with a non-zero entry point, so a bad kernel fails here
//!    with a readable error instead of silently producing an unbootable
//!    disk.
//! 2. **Build the disk image** ([`build_disk_image`]) — wraps the
//!    `bootloader` crate's `DiskImageBuilder`, which embeds its own
//!    UEFI bootloader binary (`efi/boot/bootx64.efi`) and BIOS stages
//!    and lays them out on a GPT (UEFI) or MBR (BIOS) disk together with
//!    the kernel file.
//!
//! # The `bootloader` vs `bootloader_api` split (0.11)
//!
//! In `bootloader` 0.11 the crates were split in two:
//!
//! * `bootloader` (this crate's dependency) is a **host-side** tool. It
//!   runs on your development machine and produces disk images. It needs
//!   `std`, and its build script compiles its UEFI/BIOS stages with a
//!   nightly toolchain + `rust-src` (see `rustos-toolchain`).
//! * `bootloader_api` is the **kernel-side** interface: the
//!   `entry_point!` macro and the `BootInfo` struct the kernel receives.
//!   The kernel (`rustos-kernel`) depends on that crate, not this one.
//!
//! Older tutorials used `bootloader` 0.9/0.10, where one crate did both
//! jobs and the kernel wrote `bootloader::entry_point!(...)`. That API no
//! longer exists in 0.11 — code written against it will not compile.
//! See `docs/BOOT.md`.

use std::fmt;
use std::fs;
use std::path::Path;

pub mod uefi;

pub use uefi::UefiBootInfo;

/// What [`validate_kernel_elf`] learned about a kernel file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelElf {
    /// Entry-point virtual address from the ELF header (`e_entry`).
    pub entry_point: u64,
    /// ELF machine field; always `0x3E` (`EM_X86_64`) for a valid kernel.
    pub machine: u16,
}

/// Why a file is not a usable RustOS kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfError {
    /// Fewer than 64 bytes — not even a full ELF64 header.
    TooShort(usize),
    /// First four bytes are not `0x7F 'E' 'L' 'F'`.
    BadMagic([u8; 4]),
    /// `EI_CLASS` is not `ELFCLASS64` (2).
    Not64Bit(u8),
    /// `EI_DATA` is not little-endian (1). x86-64 is little-endian only.
    NotLittleEndian(u8),
    /// `e_machine` is not `EM_X86_64` (0x3E).
    WrongMachine(u16),
    /// `e_entry` is zero — there is nowhere to jump to.
    ZeroEntryPoint,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElfError::TooShort(len) => write!(
                f,
                "file is {len} bytes, shorter than a 64-byte ELF64 header"
            ),
            ElfError::BadMagic(m) => write!(
                f,
                "bad ELF magic: expected 7F 45 4C 46, got {:02X} {:02X} {:02X} {:02X}",
                m[0], m[1], m[2], m[3]
            ),
            ElfError::Not64Bit(class) => {
                write!(f, "ELF class {class} is not 64-bit (expected ELFCLASS64 = 2)")
            }
            ElfError::NotLittleEndian(data) => write!(
                f,
                "ELF data encoding {data} is not little-endian (expected 1)"
            ),
            ElfError::WrongMachine(machine) => write!(
                f,
                "ELF machine {machine:#x} is not x86-64 (expected EM_X86_64 = 0x3e)"
            ),
            ElfError::ZeroEntryPoint => write!(f, "ELF entry point is zero"),
        }
    }
}

impl std::error::Error for ElfError {}

/// Validate the ELF64 header of a kernel image.
///
/// Reads only the 64-byte ELF header: magic, 64-bit class, little-endian
/// encoding, `EM_X86_64` machine, and a non-zero entry point. The rest of
/// the ELF (program headers, sections) is the bootloader's job at boot.
pub fn validate_kernel_elf(bytes: &[u8]) -> Result<KernelElf, ElfError> {
    if bytes.len() < 64 {
        return Err(ElfError::TooShort(bytes.len()));
    }
    if bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        return Err(ElfError::BadMagic([bytes[0], bytes[1], bytes[2], bytes[3]]));
    }
    // e_ident: [0]=0x7F, [1]=E, [2]=L, [3]=F, [4]=EI_CLASS, [5]=EI_DATA.
    let class = bytes[4];
    if class != 2 {
        return Err(ElfError::Not64Bit(class));
    }
    let data = bytes[5];
    if data != 1 {
        return Err(ElfError::NotLittleEndian(data));
    }
    // e_type (u16 @ 16), e_machine (u16 @ 18), e_version (u32 @ 20),
    // e_entry (u64 @ 24) — all little-endian on x86-64.
    let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    if machine != 0x3e {
        return Err(ElfError::WrongMachine(machine));
    }
    let entry_point = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
    if entry_point == 0 {
        return Err(ElfError::ZeroEntryPoint);
    }
    Ok(KernelElf {
        entry_point,
        machine,
    })
}

/// Build a bootable disk image containing `kernel_path`.
///
/// * `uefi = true` — GPT disk: the ESP holds `efi/boot/bootx64.efi`,
///   `kernel-x86_64`, and `boot.json`. Boot with OVMF / UEFI QEMU.
/// * `uefi = false` — MBR/BIOS disk with the bootloader's BIOS stages.
///   Boot with SeaBIOS QEMU.
///
/// The kernel file is validated with [`validate_kernel_elf`] first.
/// The image is written to `out_path` (overwritten if it exists).
///
/// This shells out to nothing itself, but building this crate compiles
/// the `bootloader` crate, whose build script builds its UEFI/BIOS
/// stages with a nightly toolchain and `rust-src` — make sure those are
/// installed (`rustos-toolchain check`).
pub fn build_disk_image(
    kernel_path: &Path,
    out_path: &Path,
    uefi: bool,
) -> anyhow::Result<KernelElf> {
    let bytes = fs::read(kernel_path).map_err(|e| {
        anyhow::anyhow!("cannot read kernel {}: {e}", kernel_path.display())
    })?;
    let kernel = validate_kernel_elf(&bytes).map_err(|e| {
        anyhow::anyhow!(
            "kernel {} is not a valid RustOS kernel: {e}",
            kernel_path.display()
        )
    })?;

    let builder = bootloader::DiskImageBuilder::new(kernel_path.to_path_buf());
    if uefi {
        builder
            .create_uefi_image(out_path)
            .map_err(|e| anyhow::anyhow!("failed to create UEFI disk image: {e:#}"))?;
    } else {
        builder
            .create_bios_image(out_path)
            .map_err(|e| anyhow::anyhow!("failed to create BIOS disk image: {e:#}"))?;
    }
    Ok(kernel)
}

/// Build only the UEFI FAT partition (no GPT wrapper).
///
/// Useful for inspecting the ESP contents or for TFTP/PXE booting.
/// The partition image is written to `out_path`.
pub fn build_uefi_fat_partition(kernel_path: &Path, out_path: &Path) -> anyhow::Result<KernelElf> {
    let bytes = fs::read(kernel_path).map_err(|e| {
        anyhow::anyhow!("cannot read kernel {}: {e}", kernel_path.display())
    })?;
    let kernel = validate_kernel_elf(&bytes).map_err(|e| {
        anyhow::anyhow!(
            "kernel {} is not a valid RustOS kernel: {e}",
            kernel_path.display()
        )
    })?;
    bootloader::DiskImageBuilder::new(kernel_path.to_path_buf())
        .create_uefi_fat_partition(out_path)
        .map_err(|e| anyhow::anyhow!("failed to create UEFI FAT partition: {e:#}"))?;
    Ok(kernel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, well-formed 64-byte ELF64 header for tests.
    fn valid_elf_header() -> Vec<u8> {
        let mut header = vec![0u8; 64];
        header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        header[4] = 2; // ELFCLASS64
        header[5] = 1; // little-endian
        header[6] = 1; // EI_VERSION
        header[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        header[18..20].copy_from_slice(&0x3eu16.to_le_bytes()); // EM_X86_64
        header[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
        header[24..32].copy_from_slice(&0x1000u64.to_le_bytes()); // e_entry
        header
    }

    #[test]
    fn accepts_a_valid_kernel_elf() {
        let kernel = validate_kernel_elf(&valid_elf_header()).unwrap();
        assert_eq!(kernel.entry_point, 0x1000);
        assert_eq!(kernel.machine, 0x3e);
    }

    #[test]
    fn rejects_a_short_file() {
        assert_eq!(validate_kernel_elf(&[0x7f, b'E']).unwrap_err(), ElfError::TooShort(2));
        assert_eq!(validate_kernel_elf(&[]).unwrap_err(), ElfError::TooShort(0));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut header = valid_elf_header();
        header[1] = b'X';
        assert_eq!(
            validate_kernel_elf(&header).unwrap_err(),
            ElfError::BadMagic([0x7f, b'X', b'L', b'F'])
        );
    }

    #[test]
    fn rejects_32_bit_elf() {
        let mut header = valid_elf_header();
        header[4] = 1;
        assert_eq!(validate_kernel_elf(&header).unwrap_err(), ElfError::Not64Bit(1));
    }

    #[test]
    fn rejects_big_endian_elf() {
        let mut header = valid_elf_header();
        header[5] = 2;
        assert_eq!(
            validate_kernel_elf(&header).unwrap_err(),
            ElfError::NotLittleEndian(2)
        );
    }

    #[test]
    fn rejects_wrong_machine() {
        let mut header = valid_elf_header();
        header[18..20].copy_from_slice(&0x28u16.to_le_bytes()); // EM_ARM
        assert_eq!(
            validate_kernel_elf(&header).unwrap_err(),
            ElfError::WrongMachine(0x28)
        );
    }

    #[test]
    fn rejects_zero_entry_point() {
        let mut header = valid_elf_header();
        header[24..32].copy_from_slice(&0u64.to_le_bytes());
        assert_eq!(
            validate_kernel_elf(&header).unwrap_err(),
            ElfError::ZeroEntryPoint
        );
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert!(ElfError::BadMagic([0, 0, 0, 0]).to_string().contains("bad ELF magic"));
        assert!(ElfError::WrongMachine(0x28).to_string().contains("x86-64"));
    }

    #[test]
    fn build_uefi_image_from_a_dummy_kernel() {
        // The builder embeds the file bytes; it does not need a fully
        // linkable kernel for image construction, but the ELF header must
        // pass our own validation first.
        let dir = std::env::temp_dir().join("rustos-bootloader-test");
        let _ = std::fs::create_dir_all(&dir);
        let kernel_path = dir.join("kernel-x86_64");
        let out_path = dir.join("disk.img");
        std::fs::write(&kernel_path, valid_elf_header()).unwrap();
        let _ = std::fs::remove_file(&out_path);
        let kernel = build_disk_image(&kernel_path, &out_path, true).unwrap();
        assert_eq!(kernel.entry_point, 0x1000);
        let image = std::fs::read(&out_path).unwrap();
        assert!(image.len() > 512, "disk image unexpectedly small");
        // A GPT disk has the "EFI PART" header at LBA 1 (byte offset 512).
        assert_eq!(&image[512..520], b"EFI PART");
        let _ = std::fs::remove_file(&kernel_path);
        let _ = std::fs::remove_file(&out_path);
    }
}
