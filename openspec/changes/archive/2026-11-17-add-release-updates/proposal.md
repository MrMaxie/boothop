## Why

Users need downloadable releases for Windows and Linux and a way to check for and install updates from the command line.

## What Changes

- Add an on-demand GitHub Actions release workflow for Windows x64 and Linux x64.
- Publish versioned archives and checksums to public releases in `MrMaxie/boothop`.
- Add `update` to check for newer stable releases and `update --install` to replace the executable.
- Share update checking with doctor and report repository access failures explicitly.

## Capabilities

### New Capabilities

- `release-updates`: Release publication, version checks, and executable replacement.

### Modified Capabilities

## Impact

Depends on the CLI foundation. Adds a release workflow, HTTP/version checking, download verification, and platform replacement adapters. Doctor consumes the update checker. Publication occurs when the user manually dispatches the future workflow.

## Archive Information

- Archived: 2026-11-17
- Outcome: Implemented and accepted by the user for version 1.0.0.
- Requirements merged into the corresponding main specification.
