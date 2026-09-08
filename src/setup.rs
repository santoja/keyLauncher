use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use crate::state::STATE_FILE;

const GROUP_NAME: &str = "keylauncher";
const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/71-keylauncher.rules";
const POLKIT_POLICY_PATH: &str =
    "/usr/share/polkit-1/actions/io.github.santoja.keylauncher.manage.policy";
const STATE_DIR: &str = "/var/lib/keylauncher";

fn invoking_username() -> std::io::Result<String> {
    // pkexec sets PKEXEC_UID; plain `sudo` sets SUDO_USER. Prefer whichever is present.
    if let Ok(user) = std::env::var("SUDO_USER") {
        return Ok(user);
    }
    if let Ok(uid) = std::env::var("PKEXEC_UID") {
        let out = Command::new("id").arg("-un").arg(&uid).output()?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }
    Err(std::io::Error::other(
        "could not determine invoking user (expected SUDO_USER or PKEXEC_UID); run `keylauncher setup` via sudo or pkexec",
    ))
}

fn ensure_group() -> std::io::Result<()> {
    let status = Command::new("groupadd").arg("-f").arg(GROUP_NAME).status()?;
    if !status.success() {
        return Err(std::io::Error::other("groupadd failed"));
    }
    Ok(())
}

fn add_user_to_group(user: &str) -> std::io::Result<()> {
    let status = Command::new("usermod")
        .arg("-aG")
        .arg(GROUP_NAME)
        .arg(user)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("usermod failed"));
    }
    Ok(())
}

fn udev_rule_content(exe: &Path) -> String {
    format!(
        "ACTION==\"add\", SUBSYSTEM==\"hidraw\", ATTRS{{idVendor}}==\"3434\", GROUP=\"{group}\", RUN+=\"{exe} __udev-apply $env{{DEVNAME}}\"\n",
        group = GROUP_NAME,
        exe = exe.display(),
    )
}

fn install_udev_rule(exe: &Path) -> std::io::Result<()> {
    std::fs::write(UDEV_RULE_PATH, udev_rule_content(exe))
}

fn polkit_policy_content(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
<policyconfig>
  <action id="io.github.santoja.keylauncher.manage">
    <description>Manage Keychron browser access</description>
    <message>Authentication is required to change Keychron device access</message>
    <defaults>
      <allow_any>auth_admin</allow_any>
      <allow_inactive>auth_admin</allow_inactive>
      <allow_active>auth_admin_keep</allow_active>
    </defaults>
    <annotate key="org.freedesktop.policykit.exec.path">{exe}</annotate>
  </action>
</policyconfig>
"#,
        exe = exe.display(),
    )
}

fn install_polkit_policy(exe: &Path) -> std::io::Result<()> {
    std::fs::write(POLKIT_POLICY_PATH, polkit_policy_content(exe))
}

fn ensure_state_file() -> std::io::Result<()> {
    std::fs::create_dir_all(STATE_DIR)?;
    let path = Path::new(STATE_FILE);
    if !path.exists() {
        crate::state::StateFile::default().save(path)?;
    }
    // World-readable: `list` doesn't need it, but a future GUI/debugging does.
    std::fs::set_permissions(STATE_FILE, std::fs::Permissions::from_mode(0o644))
}

fn reload_udev() -> std::io::Result<()> {
    let reload = Command::new("udevadm").arg("control").arg("--reload-rules").status()?;
    if !reload.success() {
        return Err(std::io::Error::other("udevadm control --reload-rules failed"));
    }
    let trigger = Command::new("udevadm")
        .args(["trigger", "--subsystem-match=hidraw", "-p", "ID_VENDOR_ID=3434"])
        .status()?;
    if !trigger.success() {
        return Err(std::io::Error::other("udevadm trigger failed"));
    }
    Ok(())
}

/// Must run as root. Idempotent: safe to re-run (e.g. after moving the binary).
pub fn run() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let user = invoking_username()?;

    ensure_group()?;
    add_user_to_group(&user)?;
    install_udev_rule(&exe)?;
    install_polkit_policy(&exe)?;
    ensure_state_file()?;
    reload_udev()?;

    println!("Setup complete. Log out and back in for the '{GROUP_NAME}' group membership to take effect.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udev_rule_content_targets_keychron_vid_and_group() {
        let content = udev_rule_content(Path::new("/opt/keylauncher"));
        assert!(content.contains("idVendor}==\"3434\""));
        assert!(content.contains("GROUP=\"keylauncher\""));
        assert!(content.contains("RUN+=\"/opt/keylauncher __udev-apply $env{DEVNAME}\""));
    }

    #[test]
    fn polkit_policy_content_has_action_id_and_exec_path() {
        let content = polkit_policy_content(Path::new("/opt/keylauncher"));
        assert!(content.contains(r#"<action id="io.github.santoja.keylauncher.manage">"#));
        assert!(content.contains(
            r#"<annotate key="org.freedesktop.policykit.exec.path">/opt/keylauncher</annotate>"#
        ));
    }
}
