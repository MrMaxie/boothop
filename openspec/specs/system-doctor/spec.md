# system-doctor Specification

## Purpose
Report available system capabilities, missing prerequisites, and update status.

## Requirements
### Requirement: Discover usable mechanisms

Boothop SHALL detect usable mechanisms for its capabilities on Windows and Linux, including systemd and OpenRC. It SHALL prefer suitable Cargo libraries and native APIs and SHALL verify any required external tool before use. When a preferred tool is absent, it SHALL detect equivalent supported alternatives.

#### Scenario: Distribution uses an alternative tool

- **WHEN** the preferred executable is missing but a compatible alternative is available
- **THEN** the capability uses the verified alternative and remains available

#### Scenario: Executable exists but lacks the operation

- **WHEN** a tool is present but cannot perform the required operation
- **THEN** detection tries supported equivalents or reports the capability unavailable with the reason

### Requirement: Actionable local diagnosis

`boothop doctor` SHALL report readiness of boot discovery, disk inspection, target selection, shutdown policies, and confirmation. It SHALL identify indispensable missing dependencies and applicable recovery actions. Permission and interaction failures SHALL be distinguished from missing dependencies. `-v` and `-vv` SHALL provide increasing diagnostic detail.

#### Scenario: No substitute for a missing dependency

- **WHEN** an indispensable dependency is absent and no equivalent is usable
- **THEN** doctor names it, identifies affected operations, and explains how to satisfy the prerequisite on the detected environment

#### Scenario: Privilege is insufficient

- **WHEN** an installed mechanism cannot be used with current privileges
- **THEN** doctor reports the required privilege rather than claiming the dependency is missing

### Requirement: Read-only bounded diagnostics

Doctor and capability probes SHALL be read-only and bounded. They SHALL NOT schedule restart, write boot state, change disk state, install dependencies, or install updates.

#### Scenario: Probe fails or hangs

- **WHEN** a capability probe errors or exceeds its time bound
- **THEN** doctor reports that check as unavailable or undetermined and continues independent checks

### Requirement: Update status in doctor

Doctor SHALL check for updates through the release update capability and distinguish available, current, and undetermined update status. Failure to access GitHub SHALL preserve local diagnostic results.

#### Scenario: Newer release exists

- **WHEN** the public repository has a newer stable compatible release
- **THEN** doctor identifies the available version and the `update --install` action

#### Scenario: Repository or network unavailable

- **WHEN** the update check cannot access the repository or network
- **THEN** doctor reports that update status could not be determined with the appropriate reason and still displays local results
