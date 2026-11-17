## Context

Disk availability, filesystem state, and Windows hibernation are separate observations. A failed probe cannot establish hibernation or a clean volume.

## Decisions

- Enumerate disks and volumes through platform adapters. Default output includes identity, disk-to-volume association, size, and filesystem. `-v` adds labels and mount locations. `-vv` adds inspection method and relevant failure details.
- Run hibernation probes only with `--check-ntfs`. Inspect each NTFS volume read-only, including evidence associated with Windows Fast Startup where it leaves a hibernated volume.
- Prefer Cargo libraries and OS APIs. If an external probe is necessary, use capability detection, explicit arguments, and documented exit statuses. Probe only mountability or metadata, without mounting or repairing the volume.
- Model results as hibernation detected, hibernation not detected, or undetermined with a reason. Keep ordinary busy, dirty, access denied, and unsupported evidence separate from positive hibernation evidence.
- Do not infer hibernation merely from the existence of `hiberfil.sys`. Validate the evidence used by each implementation against documented structures or probe semantics.
- Complete readable volumes when another volume fails and identify incomplete inspection. The result does not authorize modifying a volume or impose an additional reboot policy.

## Verification

Use synthetic metadata and recorded probe responses for hibernation, Fast Startup, clean state, busy state, dirty state, permission failure, and unreadable data. All OS access is simulated under the foundation verification boundary.
