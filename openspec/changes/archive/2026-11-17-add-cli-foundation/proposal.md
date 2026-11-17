## Why

Boothop is a command-line tool. Users need a way to discover how to invoke it and understand errors from invalid input.

## What Changes

- Show a usage overview when users start `boothop` or pass `-h` or `--help`.
- Report the installed version with `-V` or `--version`.
- Reject unsupported arguments with an actionable error.
- Provide a CLI on Windows x64 and Linux x64, including systemd and OpenRC environments.
- Establish consistent detail levels and concise dependency warnings in help.
- Support a native confirmation dialog for reboot, with terminal fallback, as specified in `add-targeted-reboot`.

## Capabilities

### New Capabilities

- `cli`: Command-line help, version reporting, argument validation, output conventions, and platform compatibility.

### Modified Capabilities

## Impact

The existing executable uses `clap`. Its basic help, version, and argument handling are implemented. The additional requirements in this proposal are planned work.

This change provides shared conventions for `add-boot-discovery`, `add-disk-inspection`, `add-targeted-reboot`, `add-system-doctor`, and `add-release-updates`.

## Archive Information

- Archived: 2026-11-17
- Outcome: Implemented and accepted by the user for version 1.0.0.
- Requirements merged into the corresponding main specification.
