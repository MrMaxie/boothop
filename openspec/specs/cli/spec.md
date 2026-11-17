# cli Specification

## Purpose
Define consistent command-line help, validation, output, and platform behavior.

## Requirements
### Requirement: CLI help

Boothop SHALL provide help that shows the command name, usage, and available options.

#### Scenario: Start with no arguments

- **WHEN** the user runs `boothop` with no arguments
- **THEN** Boothop prints help to standard output and exits successfully

#### Scenario: Request help explicitly

- **WHEN** the user runs `boothop --help` or `boothop -h`
- **THEN** Boothop prints help to standard output and exits successfully

### Requirement: CLI version

Boothop SHALL report its command name and package version.

#### Scenario: Request the version

- **WHEN** the user runs `boothop --version` or `boothop -V`
- **THEN** Boothop prints the command name and package version to standard output and exits successfully

### Requirement: Invalid CLI input

Boothop SHALL reject unsupported arguments and show users how to get help.

#### Scenario: Pass an unsupported argument

- **WHEN** the user passes an unsupported argument
- **THEN** Boothop identifies that argument on standard error, points the user to `--help`, and exits with a nonzero status

### Requirement: Supported command-line platforms

Boothop SHALL provide its command-line interface on Windows x64 and Linux x64, including systemd and OpenRC environments. The native reboot confirmation dialog SHALL be the graphical interaction described by the targeted reboot capability.

#### Scenario: Run on a supported platform

- **WHEN** the command interface is used on Windows x64, Linux x64 with systemd, or Linux x64 with OpenRC
- **THEN** the documented commands and flags have the same user-facing meaning

### Requirement: Selectable output detail

The `systems` and `disks` commands SHALL provide concise default output, descriptive details with `-v` or `--verbose`, and diagnostic details with `-vv`. Command results SHALL use stdout and warnings or errors SHALL use stderr. Output SHALL use English and remain readable without terminal colors.

#### Scenario: Select a detail level

- **WHEN** a user adds `-v` or `-vv` to a discovery command
- **THEN** the result includes the corresponding additional details without changing its identifiers or performing a different operation

#### Scenario: Redirect output

- **WHEN** output is redirected to a file
- **THEN** the text remains readable without terminal control sequences

### Requirement: Dependency warnings in help

Help SHALL warn when an indispensable local dependency is unavailable and no equivalent mechanism is usable. The warning SHALL identify the affected capability and an applicable recovery action. Help SHALL remain available and SHALL perform only bounded, read-only local checks.

#### Scenario: An indispensable dependency is missing

- **WHEN** help is requested and a required capability has no usable dependency or substitute
- **THEN** help is printed successfully and stderr contains a concise warning explaining what is unavailable

#### Scenario: A substitute is available

- **WHEN** a preferred tool is absent but an equivalent mechanism is usable
- **THEN** help does not claim the capability is unavailable

#### Scenario: Help is requested offline

- **WHEN** help is requested without network access
- **THEN** local help and dependency checks complete without querying GitHub
