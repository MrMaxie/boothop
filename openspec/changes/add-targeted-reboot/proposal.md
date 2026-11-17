## Why

Users need to restart into a discovered system without manually navigating boot menus. They also need explicit control over confirmation, scheduling, and how applications are closed.

## What Changes

- Add `boothop reboot <id>` for one-time selection of a discovered target.
- Confirm through a native dialog, with interactive terminal fallback and `--yes` to skip confirmation.
- Add `--timeout`, `--force`, and `--force-after` with distinct timing and shutdown semantics.
- Handle failures between selecting the boot target and requesting restart.

## Capabilities

### New Capabilities

- `targeted-reboot`: Confirmed one-time reboot to a selected system.

### Modified Capabilities

## Impact

Depends on `add-cli-foundation`, target resolution from `add-boot-discovery`, and capability detection from `add-system-doctor`. Requires privileged OS and boot configuration adapters. Full selection support for UEFI, Windows Boot Manager, GRUB, and systemd-boot from both Windows and Linux is a completion requirement.
