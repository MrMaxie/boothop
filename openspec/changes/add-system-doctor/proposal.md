## Why

Boot and storage capabilities depend on the host OS, installed mechanisms, and permissions. Users need to know which prerequisite prevents a requested operation and whether Boothop needs updating.

## What Changes

- Add `boothop doctor` for local readiness and update status.
- Detect usable native APIs, Cargo-backed implementations, and external tool alternatives.
- Share concise missing-dependency warnings with help and preflight checks with operations.

## Capabilities

### New Capabilities

- `system-doctor`: Capability detection, dependency fallback, and readiness reporting.

### Modified Capabilities

## Impact

Supplies local capability results to the other changes. Uses CLI conventions from `add-cli-foundation` and the shared update checker from `add-release-updates`. Local diagnostics can be implemented before update integration.
