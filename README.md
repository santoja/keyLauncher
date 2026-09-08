# keylauncher

Rust CLI bridging the Keychron Launcher web app (WebHID, browser-only) to Keychron keyboards on Linux. Based on the manual udev-rule approach from [this gist](https://gist.github.com/K0SS4/668c2f1e2dc8f8e704a679974a340bc6), replaced with a managed setup, plus a per-device enable/disable toggle that actually revokes OS-level access (not just a UI flag).

> **Unofficial project.** Not affiliated with, endorsed by, or connected to Keychron in any way.

> **Browser support.** Tested on Chrome-based browsers only. Firefox doesn't work — Keychron Launcher limitation, not this tool's.

CLI-only. A GNOME/KDE panel widget may come later in a separate repo, driven by `list --json`.

## Install

This project is in alpha and **not ready for production use**.

Download the latest release for your architecture from [Releases](https://github.com/santoja/keyLauncher/releases), verify the checksum, and install:

```
sha256sum -c keylauncher-x86_64-unknown-linux-gnu.tar.gz.sha256
tar xzf keylauncher-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 755 keylauncher /usr/local/bin/
```

### Build from source

```
cargo build --release
```

Binary lands at `target/release/keylauncher`.

## Usage

```
keylauncher setup             # one-time, privileged, self-elevates via pkexec — installs udev rule + polkit policy
keylauncher list [--json]     # unprivileged — shows detected Keychron keyboards and enabled/disabled state
keylauncher enable <id>       # privileged, self-elevates via pkexec
keylauncher disable <id>      # privileged, self-elevates via pkexec
```

Run `keylauncher setup` once after install. It adds you to a `keylauncher` group and installs the udev/polkit rules needed for `enable`/`disable` to work — log out/in afterward for the group change to take effect. `enable`/`disable`/`setup` will self-elevate through `pkexec` (a graphical auth prompt) if not already run as root.

`list` never needs root — it reads live device permission bits directly.

## Firmware updates

Flashing keyboard firmware needs `dfu-util`:

```
sudo dnf install dfu-util       # Fedora
sudo apt install dfu-util       # Debian/Ubuntu
sudo pacman -S dfu-util         # Arch
```

> **Not recommended.** Firmware update worked for us, but the Keychron Launcher website gave no indication when the flash actually finished. Proceed at your own risk.

## How it works

A udev rule grants a `keylauncher` group hidraw access to Keychron devices (USB VID `0x3434`) and re-applies saved state on every plug-in, so a `disable` survives unplug/replug instead of reverting. `enable`/`disable` are gated behind a polkit policy and flip actual file-mode permission bits on `/dev/hidrawN`, so a disabled keyboard is genuinely unreachable by the browser, not just hidden in a UI. See `CLAUDE.md` for full architecture details.

## Known limitations

- Revoking a device blocks *future* access attempts but can't close a file descriptor the browser already has open — kill/restart the browser tab if you disabled a device mid-session.
- No file locking on the state file — fine for single-user desktop use.
- Group membership changes only take effect on your next login session.

## License

MIT — see [LICENSE](LICENSE).

## Note

This project was written with the help of AI, but human tested and reviewed before release.
