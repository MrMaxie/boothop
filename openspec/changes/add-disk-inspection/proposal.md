## Why

Users need to identify disks and determine whether Windows hibernation affects their NTFS volumes before deciding how to use them.

## What Changes

- Add `boothop disks` with disk and volume information.
- Add `--check-ntfs` for read-only hibernation checks and `-v` / `-vv` detail levels.
- Report uncertain or inaccessible volumes explicitly.

## Capabilities

### New Capabilities

- `disk-inspection`: Disk inventory and read-only NTFS hibernation detection.

### Modified Capabilities

## Impact

Depends on `add-cli-foundation` and local mechanism detection from `add-system-doctor`. Requires disk enumeration and NTFS inspection adapters for Windows and Linux.
