## Why

Users need to discover the systems they can select before requesting a reboot. Stable selection identifiers connect discovery to the reboot command.

## What Changes

- Add `boothop systems` with concise output and `-v` / `-vv` details.
- Discover UEFI entries and entries in Windows Boot Manager, GRUB, and systemd-boot on both supported operating systems.
- Distinguish discovered entries, current-system identification, and selection availability.

## Capabilities

### New Capabilities

- `boot-discovery`: Boot entry discovery and selection identifiers.

### Modified Capabilities

## Impact

Depends on the CLI conventions in `add-cli-foundation` and capability detection in `add-system-doctor`. Supplies target identities and resolution to `add-targeted-reboot`. Full coverage of all four boot mechanisms is required for completion.

## Archive Information

- Archived: 2026-11-17
- Outcome: Implemented and accepted by the user for version 1.0.0.
- Requirements merged into the corresponding main specification.
