## Decisions

- Build a reusable, read-only local capability detector independently of the doctor command. Detect Windows, Linux systemd, and Linux OpenRC through available facilities rather than assumptions about distribution names.
- Prefer suitable Cargo dependencies, including Cargo git dependencies when needed, and native OS APIs. Probe external executables only when needed, check their availability and supported operation, and use equivalent distribution-specific alternatives where available.
- Execute subprocesses with explicit executable paths and argument arrays. Use bounded probes and handle unavailable, incompatible, timed-out, and failed commands. Never execute disk, boot, or shutdown mutations as a capability test.
- Keep missing dependency, insufficient privilege, unavailable session interaction, and unsupported data distinct. Report a required dependency only if no usable equivalent exists. Do not install dependencies automatically.
- Show concise per-capability readiness and actionable failures in doctor. Expose detailed backend diagnostics through `-v` and `-vv`. Help uses only indispensable-dependency warnings and does not call the network checker.
- Doctor combines local results with the same public GitHub update checker as `update`. A network failure does not suppress local results or imply that the installed version is current.
- Full supported-platform implementation is required. Runtime diagnostics for missing prerequisites do not replace unfinished systemd, OpenRC, Windows, or bootloader adapters.

## Dependencies and verification

The detector can be built first, followed by discovery and reboot integration. Complete doctor update reporting after the release checker exists. Test through injected OS, process, privilege, desktop, and network adapters under the foundation simulation-only boundary.
