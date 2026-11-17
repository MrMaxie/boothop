# targeted-reboot Specification

## Purpose
Confirm and request a one-time restart into a selected operating system while preserving persistent boot defaults.

## Requirements
### Requirement: One-time target selection

`boothop reboot <id>` SHALL accept an ID from `systems`, resolve it against current discovery, and select that target only for the next boot. It SHALL preserve persistent boot order and defaults. Selection SHALL support UEFI, Windows Boot Manager, GRUB, and systemd-boot from both host operating systems, including required firmware-to-loader selections.

#### Scenario: Reboot to a discovered target

- **WHEN** a valid target is confirmed and its required mechanisms are available
- **THEN** the necessary one-shot selections are set and the OS is asked to restart into that target

#### Scenario: Target no longer exists

- **WHEN** the supplied ID no longer identifies a discovered target
- **THEN** the command fails before any boot state or shutdown action is changed

### Requirement: Reboot confirmation

Reboot SHALL request confirmation through a simple native system dialog by default. If the dialog is unavailable, it SHALL prompt in an interactive terminal. `--yes` SHALL skip confirmation. Declining, dismissing, or failing to obtain confirmation SHALL leave boot state unchanged and SHALL NOT initiate shutdown.

#### Scenario: Native confirmation

- **WHEN** a native dialog is available
- **THEN** it identifies the target and asks the user to confirm, adding a delay only when it is nonzero and describing forced closure only when requested

#### Scenario: Immediate graceful restart confirmation

- **WHEN** a restart has no delay or forced closure
- **THEN** the confirmation asks only whether to restart into the named target, without a zero-second countdown or shutdown implementation details

#### Scenario: Windows shortcut launch

- **WHEN** a Windows shortcut invokes the shortcut launcher with `reboot <id>`
- **THEN** it starts the adjacent CLI executable without creating a console window and preserves the supplied arguments without adding `--yes`
- **AND** the native confirmation uses themed controls and per-monitor DPI scaling, with explicit Restart and Cancel buttons and cancellation selected by default
- **AND** a launch or command failure is shown in a native error dialog

#### Scenario: Terminal fallback

- **WHEN** a native dialog is unavailable and the terminal is interactive
- **THEN** equivalent confirmation is requested in the terminal with cancellation as the default

#### Scenario: Noninteractive invocation

- **WHEN** neither a dialog nor an interactive terminal is available and `--yes` is absent
- **THEN** the command stops and explains that explicit `--yes` is required

#### Scenario: Explicit unattended approval

- **WHEN** the user supplies `--yes`
- **THEN** confirmation is skipped while target and capability validation still apply

### Requirement: Separate delay and closure policy

By default reboot SHALL begin graceful shutdown immediately after confirmation. `--timeout <seconds>` SHALL delay its start. `--force` SHALL request OS-managed forced application closure. With systemd or compatible SysVinit, `--force-after <seconds>` SHALL start gracefully and escalate to OS-managed forced closure after that shutdown interval. On Windows and OpenRC with native openrc-init or BusyBox, a system-owned helper SHALL first request graceful application closure, wait the specified interval, and then request an OS-managed restart with forced application closure and normal filesystem shutdown; system service shutdown begins with that final restart request. The force modes SHALL be mutually exclusive, and automatic escalation SHALL require an explicit flag.

#### Scenario: Default graceful restart

- **WHEN** no timing or force flags are supplied
- **THEN** graceful restart begins after confirmation with no additional delay or automatic forced escalation

#### Scenario: Delay followed by escalation

- **WHEN** `--timeout 10 --force-after 60` is supplied with systemd or compatible SysVinit
- **THEN** shutdown starts after a ten-second countdown and force escalation is due sixty seconds after graceful shutdown begins if it has not completed

#### Scenario: Windows application grace period before forced restart

- **WHEN** `--timeout 10 --force-after 60` is confirmed on Windows
- **THEN** after the ten-second countdown a system-owned helper requests graceful closure of applications in logged-on user sessions
- **AND** the helper waits sixty seconds after issuing those requests, then requests a forced restart through the Windows shutdown API with normal filesystem flushing
- **AND** closing the CLI, its terminal, or a user session does not cancel the helper's timer
- **AND** Windows service shutdown begins with the final restart request, rather than at the start of the application grace period
- **AND** confirmation explains the application grace period and the eventual forced closure

#### Scenario: OpenRC application grace period with native init or BusyBox

- **WHEN** `--timeout 10 --force-after 60` is confirmed on OpenRC with native openrc-init or BusyBox
- **THEN** after the ten-second countdown an OpenRC-supervised helper requests graceful termination of user processes
- **AND** the helper waits sixty seconds, forcefully terminates remaining selected user processes, and asks OpenRC to perform its normal restart sequence
- **AND** closing the CLI or its terminal does not cancel that operation
- **AND** system service shutdown begins with the final restart request and normal filesystem shutdown is preserved
- **AND** confirmation explains the application grace period and the eventual forced closure

#### Scenario: Delayed forced restart

- **WHEN** `--timeout 10 --force` is supplied
- **THEN** OS-managed forced closure starts after the ten-second countdown

#### Scenario: Invalid timing or conflicting modes

- **WHEN** durations are invalid or unrepresentable, or both force modes are supplied
- **THEN** the command explains the invalid input and performs no mutation

#### Scenario: Session ends during graceful shutdown

- **WHEN** the CLI or its terminal session ends after an accepted `--force-after` operation
- **THEN** the platform-owned mechanism retains the requested escalation behavior

### Requirement: Preflight and partial failure recovery

Reboot SHALL verify target identity, required permissions, and all requested mechanisms before mutation. It SHALL revalidate after a countdown. Cancellation before shutdown or rejection of the restart request SHALL trigger restoration of boot state changed by this operation where possible. The command SHALL report any incomplete recovery and SHALL NOT overwrite concurrent changes.

#### Scenario: Permission or required mechanism unavailable

- **WHEN** preflight cannot establish the requested operation
- **THEN** reboot stops before changes and explains the specific prerequisite

#### Scenario: Cancellation during countdown

- **WHEN** the user cancels before shutdown begins
- **THEN** no restart is requested and any resources prepared for the operation are released

#### Scenario: A later operation fails

- **WHEN** a boot write or restart request fails after an earlier one-shot value was changed
- **THEN** prior values owned by the operation are restored and the result of recovery is reported

#### Scenario: Recovery fails or encounters a concurrent edit

- **WHEN** restoration cannot safely complete
- **THEN** the command reports the remaining changed state and recovery action without claiming rollback succeeded
