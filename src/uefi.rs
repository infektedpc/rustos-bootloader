//! UEFI boot model for RustOS.
//!
//! This module is a *pure-Rust model* of what the UEFI firmware and the
//! embedded UEFI bootloader do at boot time. It contains no UEFI runtime
//! calls: the real UEFI stage is the binary embedded in the `bootloader`
//! crate (built by its build script from its own `uefi/` sources), and the
//! real FAT/GPT work is done by [`crate::build_disk_image`].
//!
//! What this module is for:
//!
//! * Naming the files on the EFI System Partition (ESP) so the kernel,
//!   the image builder, and the docs all agree on the layout.
//! * Documenting — in code, testably — the order of operations between
//!   power-on and the kernel's `_start`.
//! * Describing the handoff (`UefiBootInfo`): what the bootloader has
//!   done for the kernel by the time it jumps to the kernel entry point.
//!
//! See `docs/BOOT.md` for the full narrative version of this flow.

/// Path of the bootloader binary inside the EFI System Partition.
///
/// UEFI firmware looks for exactly this path on a removable / GPT disk
/// when no explicit boot entry exists (the "fallback" boot path).
pub const ESP_BOOTLOADER_PATH: &str = "efi/boot/bootx64.efi";

/// File name the bootloader expects the kernel under, inside the ESP.
pub const KERNEL_FILE_NAME: &str = "kernel-x86_64";

/// Optional boot configuration file, inside the ESP next to the kernel.
pub const BOOT_CONFIG_FILE_NAME: &str = "boot.json";

/// Optional ramdisk file, inside the ESP next to the kernel.
pub const RAMDISK_FILE_NAME: &str = "ramdisk";

/// Every file the image builder places on the ESP, with its purpose.
pub fn esp_layout() -> Vec<(&'static str, &'static str)> {
    vec![
        (ESP_BOOTLOADER_PATH, "UEFI bootloader (BOOTX64.EFI)"),
        (KERNEL_FILE_NAME, "kernel ELF, loaded and jumped to"),
        (BOOT_CONFIG_FILE_NAME, "boot configuration (framebuffer, log level)"),
        (RAMDISK_FILE_NAME, "optional ramdisk image"),
    ]
}

/// The ordered steps from power-on to kernel entry, UEFI path.
///
/// 1. Firmware reads the GPT, finds the EFI System Partition (FAT).
/// 2. Firmware loads `efi/boot/bootx64.efi` and calls its UEFI entry point.
/// 3. The bootloader opens the FAT volume and reads `kernel-x86_64`.
/// 4. The bootloader parses the kernel ELF and maps its loadable segments.
/// 5. The bootloader asks UEFI for the memory map, then calls
///    `ExitBootServices` — after this, no UEFI runtime calls are safe
///    except the small runtime-services set.
/// 6. The bootloader builds the `BootInfo` struct (memory map, physical
///    memory offset, framebuffer) and jumps to the kernel entry point.
pub fn uefi_boot_steps() -> &'static [&'static str] {
    &[
        "firmware reads GPT and finds the EFI System Partition",
        "firmware loads efi/boot/bootx64.efi and calls its entry point",
        "bootloader opens the FAT volume and reads kernel-x86_64",
        "bootloader parses the kernel ELF and maps its loadable segments",
        "bootloader reads the UEFI memory map and calls ExitBootServices",
        "bootloader builds BootInfo and jumps to the kernel entry point",
    ]
}

/// What the bootloader guarantees to the kernel at handoff.
///
/// This is a host-side description, not the wire format — the wire format
/// is `bootloader_api::BootInfo`, which the kernel (see `rustos-kernel`)
/// receives as its entry-point argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UefiBootInfo {
    /// Virtual address the kernel was loaded at.
    pub kernel_entry: u64,
    /// Size of the kernel ELF image in bytes.
    pub kernel_len: u64,
    /// Number of entries in the memory map handed to the kernel.
    pub memory_map_entries: usize,
    /// Whether the bootloader mapped all physical memory for the kernel.
    pub physical_memory_mapped: bool,
    /// Whether a framebuffer was set up for the kernel.
    pub framebuffer_available: bool,
}

impl UefiBootInfo {
    /// One-line summary, used in boot logs and in tests.
    pub fn summary(&self) -> String {
        format!(
            "kernel entry {:#x} ({} bytes), {} memory-map entries, \
             physical memory mapped: {}, framebuffer: {}",
            self.kernel_entry,
            self.kernel_len,
            self.memory_map_entries,
            self.physical_memory_mapped,
            self.framebuffer_available,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esp_paths_are_the_uefi_fallback_paths() {
        assert_eq!(ESP_BOOTLOADER_PATH, "efi/boot/bootx64.efi");
        assert_eq!(KERNEL_FILE_NAME, "kernel-x86_64");
        let layout = esp_layout();
        assert!(layout.iter().any(|(p, _)| *p == ESP_BOOTLOADER_PATH));
        assert!(layout.iter().any(|(p, _)| *p == KERNEL_FILE_NAME));
    }

    #[test]
    fn boot_steps_end_at_kernel_entry() {
        let steps = uefi_boot_steps();
        assert!(steps.len() >= 6);
        assert!(steps[0].contains("GPT"));
        assert!(steps[steps.len() - 1].contains("kernel entry point"));
        assert!(steps.iter().any(|s| s.contains("ExitBootServices")));
    }

    #[test]
    fn boot_info_summary_mentions_entry_and_memory_map() {
        let info = UefiBootInfo {
            kernel_entry: 0x1000,
            kernel_len: 4096,
            memory_map_entries: 7,
            physical_memory_mapped: true,
            framebuffer_available: false,
        };
        let summary = info.summary();
        assert!(summary.contains("0x1000"));
        assert!(summary.contains("7 memory-map entries"));
    }
}
