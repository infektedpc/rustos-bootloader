# RustOS patch to vendored `uefi` 0.38.0

This is a verbatim copy of the `uefi` 0.38.0 crate from crates.io
(kept LICENSE-APACHE, LICENSE-MIT, README.md, CHANGELOG.md), with
exactly one behavioural change:

## `src/boot.rs` — `exit_boot_services` retries until success

Upstream retries the GetMemoryMap + ExitBootServices sequence twice
and then deliberately cold-resets the machine
(`log::warn!("Resetting the machine"); runtime::reset(COLD, ...)`).
On OVMF/QEMU the memory-map key frequently goes stale between the
two calls (firmware-side map changes), so the machine resets and the
bootloader cycles after "exiting boot services" until one cycle's key
happens to stay current — the boot loop.

The patch retries up to `MAX_EXIT_BOOT_SERVICES_ATTEMPTS` (100)
times, logging each failure at error level
(`ExitBootServices attempt N/100 failed: ...`), and only resets
after the cap is exhausted. This matches the Linux kernel, which
retries in a loop.

## Re-applying on upgrade

When upgrading `uefi`, copy the new crate source over this directory
(keeping this file), re-apply the change in `src/boot.rs` (search for
`RUSTOS PATCH`), and rebuild. The diff against the pristine crate is
`diff -r <registry uefi-0.38.0> vendor/uefi` — it should show only
`src/boot.rs` (plus this file).
