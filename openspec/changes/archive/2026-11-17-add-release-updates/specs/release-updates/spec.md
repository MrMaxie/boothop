## ADDED Requirements

### Requirement: On-demand releases

A GitHub Actions `workflow_dispatch` workflow SHALL build Windows x64 and Linux x64 release artifacts for an explicitly selected existing tag matching the package version. It SHALL publish both archives and their checksums only after both platforms pass build and simulated verification.

#### Scenario: Publish a complete release

- **WHEN** the workflow is dispatched for a valid version tag and both platform jobs succeed
- **THEN** the public release contains the matching Windows and Linux archives and checksum manifest

#### Scenario: Windows shortcut launcher distribution

- **WHEN** a release is packaged
- **THEN** it also provides a separate Windows shortcut launcher covered by the checksum manifest
- **AND** the Windows archive retains exactly one CLI executable so existing self-update clients remain compatible

#### Scenario: Version mismatch or platform failure

- **WHEN** the selected tag disagrees with the package version or either platform job fails
- **THEN** the workflow fails without publishing a new stable release

#### Scenario: Version is already published

- **WHEN** the selected version already has a published release
- **THEN** publication stops without replacing its assets

### Requirement: Check stable updates

`boothop update` and doctor SHALL use the same checker for public releases in `MrMaxie/boothop`. The checker SHALL compare semantic versions, select stable releases, and verify that a compatible platform asset exists. Checking SHALL NOT replace the executable.

#### Scenario: Newer compatible stable release

- **WHEN** GitHub provides a newer stable release with the matching platform asset
- **THEN** the result identifies the available version and how to install it

#### Scenario: No newer stable version

- **WHEN** the installed version is at least as recent as the available stable version
- **THEN** the result reports that no update is required

#### Scenario: Repository cannot be accessed

- **WHEN** the repository cannot be accessed through the public release API
- **THEN** the result explains that the repository is unavailable or inaccessible rather than reporting the application current

#### Scenario: No release or missing platform asset

- **WHEN** the repository is accessible but no stable release or compatible asset is available
- **THEN** the result identifies that specific condition

#### Scenario: Network or API failure

- **WHEN** the request fails due to network, API limits, or invalid response data
- **THEN** update status is undetermined and the reason is reported

### Requirement: Verified self-update

`boothop update --install` SHALL download the newer compatible release, verify its checksum and expected archive contents, and replace the installed executable using a platform-compatible recoverable procedure. It SHALL support the Windows running-file constraint and preserve Linux executable permissions.

#### Scenario: Install a valid update

- **WHEN** a newer compatible archive passes verification and the installation path is writable
- **THEN** replacement installs that version using the platform procedure and reports whether replacement is scheduled or complete

#### Scenario: Nothing to install

- **WHEN** no newer version is available
- **THEN** the command reports the result without replacing the executable

#### Scenario: Download or verification failure

- **WHEN** downloading fails, a checksum differs, or archive contents are invalid
- **THEN** installation stops before modifying the installed executable

#### Scenario: Replacement fails

- **WHEN** file permissions, locks, or another replacement operation prevent installation
- **THEN** the old executable is preserved or restored, the failure is reported, and any incomplete recovery is identified
