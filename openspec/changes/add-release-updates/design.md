## Release contract

- Use `workflow_dispatch` with a required existing tag in `v<package-version>` form. Check out that exact tag and verify its version against Cargo metadata before building.
- Build with the mise-pinned Rust toolchain and locked dependencies. Run formatting, Clippy, and simulated tests. Agent and CI must not execute the production binary, including smoke checks using help or version flags.
- Produce `boothop-v<version>-windows-x86_64.zip` containing `boothop.exe` and `boothop-v<version>-linux-x86_64.tar.gz` containing `boothop`. Prefer a musl Linux target to support both glibc and musl distributions, including OpenRC systems, without an additional application runtime installation. Validate the resulting runtime dependency set as part of packaging.
- Produce `SHA256SUMS` covering both archives. Stage artifacts from both successful platform jobs, then publish the stable release only once the complete asset set and verification are ready. Fail without publishing a new stable release if either build fails.
- Refuse to overwrite an already published version. Recovering an incomplete unpublished draft is permitted only for the same tag and verified artifacts.

## Update contract

- Query public GitHub releases for `MrMaxie/boothop` over HTTPS without requiring authentication. Compare semantic versions with the compiled Cargo package version and select a newer stable release with the asset matching the running platform.
- Keep HTTP transport, release parsing, version comparison, asset selection, and replacement separate so fixtures can exercise the entire orchestration without real GitHub access or host replacement.
- `update` only checks. Report a newer compatible version, current version, unavailable repository, no published stable release, missing platform asset, or network/API failure as appropriate. Repository access failure must not be presented as proof that the repository does not exist.
- `update --install` uses the same check. If no newer version is available, report that result without replacing the binary. Otherwise download to temporary storage, verify the release checksum, and validate archive contents before touching the installed executable.
- Extract only the expected executable, reject path traversal or unexpected executable identity, and preserve Linux executable permissions. Check write permission to the installation location before replacement and provide a recovery action on failure.
- On Linux use a staged file on the destination filesystem and recoverable rename. On Windows use a platform-compatible replacement helper that waits for the running executable to release its file. Keep the previous executable recoverable until replacement succeeds. Report scheduled replacement separately from completed replacement.
- Serialize replacements of the same executable and clean task-owned temporary files. If replacement fails, preserve or restore the old executable and report any recovery failure. Tests operate only on disposable files and simulated process/file-lock behavior.

## Verification and manual acceptance

Test GitHub responses, version ordering, asset names, absent repositories, no releases, network failures, malformed archives, checksum mismatch, locked files, denied writes, concurrent installs, and recovery. Builds and test harnesses are allowed. Real self-replacement is manually verified by the user after implementation.
