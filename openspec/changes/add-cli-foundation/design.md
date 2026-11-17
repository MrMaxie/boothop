## Context

The executable already uses `clap` for help, version reporting, and argument errors. This proposal extends that foundation into the shared CLI contract for the planned capabilities.

## Goals / Non-Goals

Establish the command-line entry point, output conventions, and platform contract. Feature commands have their own specifications.

## Decisions

- Use `clap`'s command builder. It provides standard help, terminal styling, and argument errors. The builder keeps the root command small while it has only built-in options.
- Read the command name and version from Cargo package metadata so the help and version output follow the manifest.
- Show help on an empty invocation so users can discover the available options on their first run.
- Register `systems`, `disks`, `reboot`, `doctor`, and `update` as their capabilities are implemented.
- Use English output and ordinary keyboard punctuation. Default lists contain fields needed to select a system or identify a disk. `-v` adds descriptive detail and `-vv` adds relevant diagnostics. Preserve identifiers that users need to pass to commands.
- Send command results to stdout and warnings/errors to stderr. Keep help readable when piped, with automatic terminal styling. Local capability checks used by help must be read-only, bounded, and independent of network access.
- Share capability detection between help, doctor, discovery, and operations. `add-system-doctor` owns the detector and fallback policy. Access denied and a missing dependency are distinct outcomes.
- Support Windows x64 and Linux x64 with systemd and OpenRC. Full bootloader discovery and selection coverage is required for UEFI, Windows Boot Manager, GRUB, and systemd-boot from both operating systems where those boot configurations exist. An unimplemented adapter remains unfinished work.

## Verification boundary

All new automated behavior checks use injected system adapters, fixtures, and a controlled clock. Agent and CI verification must not execute the production application, including its help command, or invoke host boot, shutdown, disk mutation, or executable replacement operations. Build and static checks are permitted. Exercise command parsing and orchestration in a test harness wired exclusively to simulated adapters.

The user performs manual verification of actual system behavior after implementation. Record simulated checks and manual checks separately. Earlier completed foundation tasks describe historical verification and do not authorize repeating live execution.
