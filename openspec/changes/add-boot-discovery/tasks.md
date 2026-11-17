## 1. Discovery

- [x] 1.1 Implement source-qualified identities and fresh target resolution shared with reboot planning.
- [x] 1.2 Implement read-only discovery for UEFI and Windows Boot Manager from both Windows and Linux.
- [x] 1.3 Implement read-only discovery for GRUB and systemd-boot from both Windows and Linux, including nested boot paths.
- [x] 1.4 Integrate `systems`, detail levels, current-system evidence, and partial-result reporting.
- [x] 1.5 Read Linux boot files directly on Windows from ext2/3/4, single-device Btrfs, and XFS partitions, including Btrfs subvolumes and compressed files.

## 2. Verification

- [x] 2.1 Simulate all four boot sources on both host platforms, duplicate names, reordered entries, stale IDs, malformed data, and access failures.
- [x] 2.2 Verify discovery performs no writes or configuration execution through the simulated system boundary and read-only parser interfaces.
- [x] 2.4 Verify raw filesystem readers against synthetic mkfs images on Windows and Linux, including partition boundaries, symlinks, corruption, and stable identities.
- [ ] 2.3 Run static checks and simulated tests, then record the user's manual discovery results separately.

## Implementation evidence

UEFI, BCD, literal GRUB menus, systemd-boot text entries, and UKI metadata have implementations. Windows additionally reads Linux boot files through read-only raw filesystem readers, tested on synthetic ext2/3/4, Btrfs, and XFS images on both platforms. Fixture and command simulations cover partial results, duplicate labels, reordered entries, stale IDs, malformed records, EFI device paths, and literal GRUB redirects. Current-system marking requires a unique entry and matching firmware path. Inaccessible files and unprovable loader chains remain explicit partial or unavailable results. Direct selection of raw Linux menu entries still requires a writable, verified bootloader path. Native OS enumeration and actual bootability require manual acceptance.
