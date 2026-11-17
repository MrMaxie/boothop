## 1. Inventory and inspection

- [x] 1.1 Implement disk and volume inventory adapters for Windows and Linux.
- [x] 1.2 Implement read-only NTFS hibernation probes and distinguish positive, negative, and undetermined results.
- [x] 1.3 Integrate `disks`, `--check-ntfs`, detail levels, and partial-result reporting.

## 2. Verification

- [x] 2.1 Simulate hibernation and Fast Startup header evidence, negative evidence, busy and dirty volumes, permission failures, and malformed data on both platforms.
- [x] 2.2 Assert that inspection invokes no mounting, repair, or write operations.
- [x] 2.3 Run static checks and simulated tests, then record the user's manual NTFS inspection results separately.

## Implementation evidence

Windows and Linux inventories and the read-only NTFS parser are implemented. Tests use an in-memory NTFS image, synthetic hibernation headers, and simulated Windows/Linux volume identities. Mounted volumes are not probed, permission errors remain undetermined, positive evidence survives a dirty-volume flag, and partial results are retained. Live NTFS inspection is covered by user acceptance below, separately from simulations.

## Manual acceptance

On 2026-11-17, the user confirmed that the implementation works and approved closing the OpenSpec changes for version 1.0.0. This records user-reported acceptance, separately from automated simulation results.
