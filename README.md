# Boothop

Boothop is a tool for switching between operating systems on a dual-boot or
multi-boot computer. Choose the system for your next restart without changing
your default.

## Install from source

With mise installed, run from the repository:

```text
mise install
mise exec -- cargo install --path . --locked
```

## Usage

List the available systems:

```text
boothop systems
```

Restart into one of them using its ID:

```text
boothop reboot <id>
```

Boothop asks for confirmation before restarting.

## Commands

| Command | Description |
| --- | --- |
| `boothop systems` | List boot targets and their IDs. |
| `boothop disks` | List disks and volumes. Add `--check-ntfs` to check for Windows hibernation. |
| `boothop reboot <id>` | Restart into the selected target for the next boot only. |
| `boothop doctor` | Check system readiness, missing dependencies, and available updates. |
| `boothop update` | Check for a newer version. Add `--install` to install it. |
| `boothop help [command]` | Show general help or the options for a command. |

Add `-v` or `-vv` to `systems`, `disks`, or `doctor` for more detail.

## License

Licensed under the [Apache License 2.0](./LICENSE).
