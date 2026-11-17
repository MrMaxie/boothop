## 1. Selection and confirmation

- [x] 1.1 Implement parsing, fresh ID resolution, boot-path planning, and permission/capability preflight.
- [x] 1.2 Implement one-shot selection and restoration for all four boot mechanisms on Windows and Linux.
- [x] 1.3 Implement native dialogs, terminal fallback, default cancellation, and `--yes`.
- [x] 1.4 Add a Windows shortcut launcher without a console, DPI-aware native confirmation with Restart and Cancel buttons, and concise text that omits zero delay and default closure details.

## 2. Shutdown orchestration

- [x] 2.1 Implement the cancellable pre-shutdown countdown and revalidation before mutations.
- [x] 2.2 Implement graceful and OS-managed forced restart for Windows, systemd, and OpenRC.
- [x] 2.3 Implement platform-owned timed escalation that survives CLI/session termination on all supported platforms.
- [x] 2.4 Implement partial failure recovery, concurrent-change protection, and operation resource cleanup.

## 3. Verification

- [x] 3.1 Simulate every confirmation path, invalid flag combination, timing boundary, stale target, and preflight failure.
- [x] 3.2 Simulate each write/request failure point, failed recovery, retries, and CLI termination before escalation.
- [x] 3.3 Verify platform coverage and static checks without invoking any live system operation.
- [x] 3.4 Record the user's manual target-boot and shutdown-policy checks separately after implementation.
- [x] 3.5 Simulate shortcut argument forwarding and confirmation text; inspect the Windows GUI subsystem and embedded DPI/theme manifest without launching either executable.

## Implementation evidence

One-shot writes, read-back verification, reverse recovery, concurrent edit protection, cancellation, and exclusive operation locks are implemented. Windows uses the approved application-grace service. Systemd uses a transient service. OpenRC uses verified SysVinit timing or the approved supervised helper for openrc-init/BusyBox. Helper simulations cover ordering, failure boundaries, persisted deadlines, uncertain submission, duplicate-request prevention, and process identity checks. Boot paths require accessible configuration and a provable loader chain. Native API bindings, service integration, and actual restart outcomes are covered by user acceptance below; simulations do not claim to verify those live outcomes.

## Manual acceptance

On 2026-11-17, the user confirmed that the implementation works and approved closing the OpenSpec changes for version 1.0.0. This records user-reported acceptance, separately from automated simulation results.
