//! `rustos-bootloader` — command line front end.
//!
//! ```sh
//! rustos-bootloader --kernel target/x86_64-unknown-none/debug/rustos-kernel \
//!                   --out disk.img            # UEFI/GPT (default)
//! rustos-bootloader --kernel <kernel> --out disk.img --bios
//! rustos-bootloader --kernel <kernel> --out esp.img --fat
//! ```

use rustos_bootloader::{build_disk_image, build_uefi_fat_partition};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
rustos-bootloader — build a bootable RustOS disk image

USAGE:
    rustos-bootloader --kernel <path> --out <path> [--uefi|--bios] [--fat]
    rustos-bootloader --help

OPTIONS:
    --kernel <path>  Kernel ELF built for x86_64-unknown-none
                     (see rustos-kernel; must be a 64-bit x86-64 ELF).
    --out <path>     Disk image to write (overwritten if it exists).
    --uefi           GPT/UEFI image (default). Boot with OVMF QEMU.
    --bios           MBR/BIOS image. Boot with SeaBIOS QEMU.
    --fat            With --uefi: write only the FAT ESP partition,
                     instead of the GPT-wrapped disk image.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") || args.is_empty() {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let mut kernel: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut uefi = true;
    let mut fat_only = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--kernel" => {
                i += 1;
                kernel = args.get(i).map(PathBuf::from);
            }
            "--out" => {
                i += 1;
                out = args.get(i).map(PathBuf::from);
            }
            "--uefi" => uefi = true,
            "--bios" => uefi = false,
            "--fat" => fat_only = true,
            other => {
                eprintln!("rustos-bootloader: unknown argument `{other}`\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let (Some(kernel), Some(out)) = (kernel, out) else {
        eprintln!("rustos-bootloader: --kernel and --out are both required\n{USAGE}");
        return ExitCode::FAILURE;
    };

    let result = if fat_only {
        if !uefi {
            eprintln!("rustos-bootloader: --fat only makes sense with --uefi");
            return ExitCode::FAILURE;
        }
        build_uefi_fat_partition(&kernel, &out)
    } else {
        build_disk_image(&kernel, &out, uefi)
    };

    match result {
        Ok(kernel_info) => {
            let kind = if fat_only {
                "UEFI FAT partition"
            } else if uefi {
                "UEFI/GPT disk image"
            } else {
                "BIOS/MBR disk image"
            };
            println!(
                "Built {kind}: {} (kernel entry {:#x})",
                out.display(),
                kernel_info.entry_point
            );
            println!("Boot it with:");
            if fat_only {
                println!("  qemu-system-x86_64 -drive file={},format=raw -bios /usr/share/OVMF/OVMF_CODE.fd", out.display());
            } else if uefi {
                println!("  qemu-system-x86_64 -drive file={},format=raw -bios /usr/share/OVMF/OVMF_CODE.fd", out.display());
            } else {
                println!("  qemu-system-x86_64 -drive file={},format=raw", out.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rustos-bootloader: {e:#}");
            ExitCode::FAILURE
        }
    }
}
