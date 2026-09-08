# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Maintenance

After finishing a task, update this file IF something you learned would change how a future session approaches this codebase (a gotcha, a fixed bug's root cause, a new limitation). Skip the update if nothing durable changed.

## What this is

`keylauncher` is a Rust CLI that bridges the Keychron Launcher web app (which talks to keyboards over WebHID in the browser) to Keychron keyboards on Linux. It replaces the manual udev-rule approach from https://gist.github.com/K0SS4/668c2f1e2dc8f8e704a679974a340bc6 with a managed setup, and adds a per-device enable/disable toggle that actually revokes OS-level access (not just a UI flag).

This is CLI-only. A GNOME/KDE panel widget is a planned future phase, likely in a separate repo, that will drive this CLI (`list --json` is the intended integration point). Do not add GUI/panel code here unless asked.

## Commands

```
cargo build              # debug build -> target/debug/keylauncher
cargo build --release    # release build -> target/release/keylauncher
cargo test                # run all unit tests
cargo test <name>          # run a single test by (substring) name, e.g. `cargo test groups_composite`
cargo run -- list --json    # run without building first
```

No lint/format tooling is configured beyond stock `cargo build` warnings — there's no clippy/rustfmt CI step to run here.

## CLI surface

```
keylauncher list [--json]     # unprivileged
keylauncher enable <id>       # privileged, self-elevates via pkexec
keylauncher disable <id>      # privileged, self-elevates via pkexec
keylauncher setup             # privileged, one-time system install (explicit only, never auto-triggered)

# hidden (not in --help), internal entry points only:
keylauncher __privileged set-state <id> <true|false>   # invoked via pkexec by enable/disable
keylauncher __privileged setup                          # invoked via pkexec by `setup`
keylauncher __udev-apply <devnode>                        # invoked directly by udev on device ADD (already root)
```

## Architecture

Four modules, each with one job:

- **`src/device.rs`** — device discovery and identity, no privileged I/O.
  - `scan_hidraw()` enumerates Keychron (USB VID `0x3434`) hidraw devices by walking `/sys/class/hidraw` directly (no `libudev`/`udev` crate — that was tried first but its `libudev-sys` build requires `systemd-devel`/pkg-config which isn't guaranteed present; pure `/sys` parsing needs zero extra system packages). For each `hidraw` node it canonicalizes the symlink and walks up ancestor directories until it finds one containing `idVendor` (the USB device node), then reads `idVendor`/`idProduct`/`serial`/`product` from there.
  - `resolve_id()` / `group_devices()` — a physical keyboard exposes **multiple hidraw nodes** (keyboard usage page + VIA/vendor config usage page); these group raw records into one `Device` per USB serial. No serial → falls back to a hash of the USB devpath (`noserial-<vid>-<pid>-<hash>`), which is stable only as long as the device stays in the same port.
  - `node_is_enabled()` / `apply_permission()` — enabled/disabled is read and written as literal file-mode bits on `/dev/hidrawN` (`0o660` = enabled, `0o600` = disabled), not tracked separately. `list` derives its enabled/disabled column by `stat()`-ing the live node, so `list` never needs root or the state file.
  - `resolve_id`, `group_devices` are pure functions with unit tests (see `#[cfg(test)]` at the bottom). `scan_hidraw` touches real `/sys` and is intentionally left untested — needs real hardware.

- **`src/state.rs`** — `StateFile` is a thin `HashMap<String, bool>` wrapper (device id → enabled), persisted as JSON at `/var/lib/keylauncher/state.json`. This is the durable record of user intent, separate from the live permission bits in `device.rs`, because it has to survive unplug/replug (see `__udev-apply` below). Unseen ids default to `enabled = true` (matches the underlying udev rule's default-allow) and are only written on an explicit `enable`/`disable`.

- **`src/setup.rs`** — everything `keylauncher setup` installs, must run as root:
  1. `groupadd -f keylauncher`, `usermod -aG keylauncher <invoking user>` (invoking user resolved from `$SUDO_USER` or `$PKEXEC_UID`, not `whoami`, since this runs elevated).
  2. Writes `/etc/udev/rules.d/71-keylauncher.rules`: on ADD for VID `3434` hidraw devices, sets `GROUP=keylauncher` and `RUN+="<abs path to this binary> __udev-apply $env{DEVNAME}"`. The binary's own path is resolved via `current_exe()` at setup time, not hardcoded — if the binary later moves, `setup` must be re-run.
  3. Writes the polkit policy to `/usr/share/polkit-1/actions/io.github.santoja.keylauncher.manage.policy`, action id `io.github.santoja.keylauncher.manage`, `exec.path` pinned to the same resolved binary path. One action covers both `set-state` and `setup`.
  4. `udevadm control --reload-rules && udevadm trigger ...` to apply immediately, including to any Keychron already plugged in.
  - Deliberately **not** using the gist's `GROUP="users"` (that group is empty under Fedora's user-private-groups default, so it silently grants nobody anything) and **not** using systemd's dynamic `uaccess` tag (logind can re-ACL a device open on session/seat changes, which would silently undo an explicit `disable`).

- **`src/main.rs`** — clap CLI wiring plus the pkexec self-elevation dance and the three subcommand handlers (`cmd_list`, `cmd_set_state`, `cmd_udev_apply`).
  - `elevate_and_run()`: if already root (e.g. run via `sudo`), dispatches directly; otherwise re-execs itself as `pkexec <current_exe> __privileged <args>`. Root check is done by `stat`-ing `/proc/self` and reading its owner uid (Linux keeps `/proc/[pid]` owned by the process's effective uid) — chosen specifically to avoid pulling in the `libc` crate just for `geteuid()`.
  - `cmd_set_state` (id, enabled): read-modify-write `state.json`, then immediately re-`chmod`s all currently-attached hidraw nodes for that id, so a toggle takes effect without waiting for a replug.
  - `cmd_udev_apply` (devnode): the reapply-on-replug path — runs as root directly (invoked by udev, no pkexec), looks up the device's persisted state and `chmod`s just that one node. This is what makes `disable` survive an unplug/replug instead of silently reverting to the udev rule's default `GROUP=keylauncher` permissions.
  - `enable`/`disable`/`setup` all check `require_setup_done()` first (does `/etc/udev/rules.d/71-keylauncher.rules` exist?) and refuse with a message pointing at `keylauncher setup` rather than silently auto-installing.

## Known, deliberate limitations

- Revoking a device's permission bits blocks *future* `open()` calls but cannot close a file descriptor the browser already has open from before the `disable` — killing the browser process is out of scope.
- No file locking on `state.json` — fine for a single-user desktop tool; would need `flock` if this ever became multi-writer.
- Group membership changes (`usermod -aG`) only take effect on the user's next login session; `setup` prints this but can't force it. **Restarting the browser is not enough** — group list is fixed at login (PAM), inherited by every process forked in that session including a freshly-relaunched browser. Check `grep Groups /proc/<pid>/status` on the browser process to confirm; if the new gid is missing, only a full logout/login or reboot fixes it.
- `reload_udev()` in `src/setup.rs` uses `udevadm trigger --subsystem-match=hidraw -p ID_VENDOR_ID=3434` (property-match), not `--attr-match=idVendor=3434`. `--attr-match` only checks the hidraw device's own sysfs attributes; `idVendor` lives on the ancestor USB device, so `--attr-match` silently matches zero devices — any Keychron already plugged in when `setup` runs never gets its `GROUP=` corrected (stays `root:root`, mode bits still get patched by `enable`/`disable` since those bypass udev, but group ownership doesn't, making the mode bits useless). Verify a trigger filter actually matches with `udevadm trigger --dry-run <same args>` before trusting it.
