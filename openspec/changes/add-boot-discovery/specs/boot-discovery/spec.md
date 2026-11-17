## ADDED Requirements

### Requirement: Discover boot targets

`boothop systems` SHALL discover available UEFI, Windows Boot Manager, GRUB, and systemd-boot entries from Windows and Linux. Default output SHALL identify each target by a reusable ID and label and mark the current system when established.

#### Scenario: Discover supported boot mechanisms

- **WHEN** readable boot entries exist for any of the four supported mechanisms on either host OS
- **THEN** the corresponding entries are listed with reusable IDs and labels

#### Scenario: Current system cannot be established

- **WHEN** discovery cannot reliably associate the running system with a listed entry
- **THEN** the list does not assign an inferred current marker

#### Scenario: Linux boot files on an unmounted Windows partition

- **WHEN** Windows discovers a readable ext2, ext3, ext4, single-device Btrfs, or XFS partition containing GRUB, BLS, or UKI boot files
- **THEN** discovery reads those files directly through a read-only partition handle without mounting the filesystem or requiring an installed filesystem driver
- **AND** entries retain source-qualified IDs that distinguish partitions, Btrfs subvolumes, and files
- **AND** discovering a menu does not imply that its individual entries can be selected without a verified writable boot path

#### Scenario: Btrfs subvolumes and compressed boot files

- **WHEN** boot files reside in a Btrfs subvolume and use zlib, LZO, or Zstd compression
- **THEN** discovery inspects the subvolume and decodes the files without changing their contents

#### Scenario: Raw filesystem cannot be inspected completely

- **WHEN** a recognized filesystem is corrupt, inaccessible, unsupported by its reader, or exceeds an inspection limit
- **THEN** discovery reports the affected source and reason while retaining readable entries from other sources
- **AND** no repair, journal write, mount, or boot configuration execution occurs

### Requirement: Stable unambiguous identities

Target IDs SHALL distinguish entries with identical names and remain reusable across list ordering changes. Reboot SHALL revalidate the selected identity against current discovery.

#### Scenario: Duplicate names and reordered results

- **WHEN** two distinct entries share a label and their listing order changes
- **THEN** each retains its own ID that resolves to the same entry

#### Scenario: Entry removed since discovery

- **WHEN** a previously listed ID no longer resolves
- **THEN** resolution reports that the target is unavailable without selecting another entry

### Requirement: Boot discovery detail and completeness

`-v` SHALL include bootloader and known location. `-vv` SHALL add relevant detection details. Incomplete discovery or an unavailable selection mechanism SHALL be reported with its reason. Discovery SHALL be read-only and SHALL not claim to have verified that an OS boots.

#### Scenario: Detailed listing

- **WHEN** the user requests `systems -v` or `systems -vv`
- **THEN** the requested detail level is shown with the same target IDs

#### Scenario: One source cannot be read

- **WHEN** one source is inaccessible while others can be read
- **THEN** readable entries are shown and the missing coverage is explicitly reported

#### Scenario: Empty complete result

- **WHEN** all applicable sources are inspected successfully and contain no targets
- **THEN** the result reports that no boot targets were found
