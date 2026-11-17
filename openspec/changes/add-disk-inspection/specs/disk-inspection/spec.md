## ADDED Requirements

### Requirement: List disks and volumes

`boothop disks` SHALL list disks and associated volumes on Windows and Linux, including their identifiers, sizes, and known filesystems. `-v` SHALL add labels and mount locations, and `-vv` SHALL add relevant inspection diagnostics.

#### Scenario: List a machine's storage

- **WHEN** the inventory contains multiple disks and volumes
- **THEN** users can identify each disk, its volumes, sizes, and known filesystems

#### Scenario: Request more details

- **WHEN** the user passes `-v` or `-vv`
- **THEN** the corresponding details are added without changing the inventory's identities

### Requirement: Inspect NTFS hibernation

`disks --check-ntfs` SHALL inspect each accessible NTFS volume for Windows hibernation, including applicable Fast Startup evidence, and report detected, not detected, or undetermined with a reason.

#### Scenario: Hibernation evidence is present

- **WHEN** validated volume evidence indicates Windows hibernation
- **THEN** that volume is reported as hibernated

#### Scenario: Successful negative inspection

- **WHEN** a complete supported inspection finds no hibernation evidence
- **THEN** that volume is reported as hibernation not detected

#### Scenario: Volume is busy or inaccessible

- **WHEN** a volume cannot be inspected because it is busy, permission is denied, or its data cannot be interpreted
- **THEN** the result is undetermined with the specific reason, and other readable volumes are still inspected

### Requirement: Read-only storage inspection

Disk listing and NTFS inspection SHALL leave volume contents and mount state unchanged. They SHALL NOT remove hibernation files, repair filesystems, unlock volumes, or mount them to perform the check.

#### Scenario: Inspect a hibernated volume

- **WHEN** a hibernated NTFS volume is checked
- **THEN** the result is reported without modifying its metadata, hibernation data, or mount state
