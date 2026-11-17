## 1. CLI entry point

- [x] 1.1 Add `clap` and replace the greeting with a command that provides help, version output, and argument validation.
- [x] 1.2 Check the command harness against help, version, and invalid input scenarios without executing the production binary.

## 2. Shared CLI behavior

- [x] 2.1 Add the shared output conventions and `-v` / `-vv` detail levels for discovery commands.
- [x] 2.2 Integrate read-only local capability warnings into help using the detector from `add-system-doctor`.
- [x] 2.3 Establish Windows x64 and Linux x64 platform adapters, including systemd and OpenRC support.

## 3. Verification

- [x] 3.1 Verify parser, help, output streams, detail levels, and dependency warnings through a simulated command harness.
- [x] 3.2 Run formatting, Clippy, builds, and simulated tests on both platform targets without running the production application.
- [x] 3.3 Record the user's manual platform verification separately from automated results after implementation.

## Implementation evidence

Platform code compiles on Windows and Linux. The simulated command harness covers help, version, parsing, warnings, output, cancellation, and operation dispatch. OpenRC supports verified SysVinit timing and the approved supervised application-grace policy for openrc-init and BusyBox. Production executables are never launched by automated checks. Manual acceptance is recorded below.

## Manual acceptance

On 2026-11-17, the user confirmed that the implementation works and approved closing the OpenSpec changes for version 1.0.0. This records user-reported acceptance, separately from automated simulation results.
