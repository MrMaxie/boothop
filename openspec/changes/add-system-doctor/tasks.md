## 1. Local capability detection

- [x] 1.1 Implement read-only capability detection for Windows, Linux systemd, and Linux OpenRC.
- [x] 1.2 Implement bounded external-tool detection, supported-operation checks, and distribution-specific equivalent fallbacks.
- [x] 1.3 Integrate actionable doctor results, detail levels, shared help warnings, and operation preflight.

## 2. Update integration and verification

- [x] 2.1 Reuse the release checker for doctor update status while preserving local results on network failure.
- [x] 2.2 Simulate missing and incompatible tools, alternatives, timeouts, permission failures, missing desktop sessions, and offline update checks.
- [x] 2.3 Verify probes invoke no mutations and run static checks plus simulated tests.
- [ ] 2.4 Record the user's manual environment diagnosis checks separately after implementation.

## Implementation evidence

Help and doctor share dependency warnings; doctor checks boot inventory, unavailable target paths, disk inventory, shutdown modes, privileges, confirmation, and updates. External calls have time/output bounds. Candidate selection, incompatible or timed-out probes, permissions, confirmation fallback, and offline update checks have simulations. Native tool and API integration remains subject to manual acceptance.
