## 1. Release artifacts

- [x] 1.1 Add the manually dispatched workflow with existing-tag and package-version validation.
- [x] 1.2 Build Windows x64 and portable Linux x64 release binaries with locked dependencies and verify runtime dependencies.
- [x] 1.3 Package versioned archives and checksums and publish only after both platform jobs succeed, without overwriting published releases.

## 2. Update behavior

- [x] 2.1 Implement the shared public GitHub stable-release checker and precise access/error results.
- [x] 2.2 Integrate `update`, doctor update reporting, and compatible artifact selection.
- [x] 2.3 Implement verified download and archive validation for `update --install`.
- [x] 2.4 Implement recoverable Linux and Windows replacement, permissions, locking, and cleanup.

## 3. Verification

- [x] 3.1 Simulate version comparisons, inaccessible repositories, no releases, missing assets, API failures, and checksum/archive errors.
- [x] 3.2 Simulate running Windows file locks, Linux permissions, concurrent updates, failed replacement, and recovery on disposable files.
- [x] 3.3 Validate workflow gating, artifact names, and both platform builds without running production binaries or publishing during agent verification.
- [ ] 3.4 Record the user's manual installation and replacement checks separately after implementation.

## Implementation evidence

Windows x64 and Linux x64 musl release builds are verified locally without execution. Packaging, archive checks, checksums, version/API failures, installation locking, partial activation recovery, failed recovery with backup retention, and simulated running-file locks have tests. The workflow is authored and checked locally; it has not been dispatched or published. Real running-executable replacement requires manual acceptance.
