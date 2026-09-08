mod device;
mod setup;
mod state;

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand};
use device::{apply_permission, group_devices, node_is_enabled, scan_hidraw};
use state::{StateFile, STATE_FILE};

#[derive(Parser)]
#[command(name = "keylauncher", about = "Bridge Keychron Launcher (browser/WebHID) access to Keychron keyboards")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List connected Keychron devices and whether they're accessible to the browser.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Allow browser (WebHID) access to a device.
    Enable { id: String },
    /// Revoke browser (WebHID) access to a device.
    Disable { id: String },
    /// One-time system setup: udev rule, polkit policy, keylauncher group.
    Setup,
    #[command(name = "__privileged", hide = true, subcommand)]
    Privileged(PrivilegedCommand),
    /// Invoked by udev on device add; reapplies persisted enable/disable state.
    #[command(name = "__udev-apply", hide = true)]
    UdevApply { devnode: String },
}

#[derive(Subcommand)]
enum PrivilegedCommand {
    #[command(name = "set-state")]
    SetState {
        id: String,
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },
    Setup,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::List { json } => cmd_list(json),
        Commands::Enable { id } => elevate_and_run(&["__privileged", "set-state", &id, "true"]),
        Commands::Disable { id } => elevate_and_run(&["__privileged", "set-state", &id, "false"]),
        Commands::Setup => elevate_and_run(&["__privileged", "setup"]),
        Commands::Privileged(PrivilegedCommand::SetState { id, enabled }) => {
            cmd_set_state(&id, enabled)
        }
        Commands::Privileged(PrivilegedCommand::Setup) => setup::run(),
        Commands::UdevApply { devnode } => cmd_udev_apply(&devnode),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Re-exec ourselves under pkexec unless we're already root (e.g. run via `sudo`).
fn elevate_and_run(privileged_args: &[&str]) -> std::io::Result<()> {
    if is_root() {
        // Already root: dispatch directly without a second pkexec prompt.
        return match privileged_args {
            ["__privileged", "set-state", id, enabled] => {
                cmd_set_state(id, enabled.parse().unwrap_or(true))
            }
            ["__privileged", "setup"] => setup::run(),
            _ => unreachable!(),
        };
    }

    let exe = std::env::current_exe()?;
    let status = Command::new("pkexec").arg(exe).args(privileged_args).status()?;
    if !status.success() {
        return Err(std::io::Error::other("pkexec command failed or was cancelled"));
    }
    Ok(())
}

fn is_root() -> bool {
    std::fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false)
}

fn require_setup_done() -> std::io::Result<()> {
    if !Path::new("/etc/udev/rules.d/71-keylauncher.rules").exists() {
        return Err(std::io::Error::other(
            "keylauncher is not set up yet; run `keylauncher setup` first",
        ));
    }
    Ok(())
}

fn cmd_list(json: bool) -> std::io::Result<()> {
    let records = scan_hidraw()?;
    let devices = group_devices(records);

    if json {
        let out: Vec<_> = devices
            .iter()
            .map(|d| {
                let enabled = d
                    .hidraw_nodes
                    .first()
                    .and_then(|n| node_is_enabled(n).ok())
                    .unwrap_or(false);
                serde_json::json!({
                    "id": d.id,
                    "vendor": format!("{:04x}", d.vendor),
                    "product": format!("{:04x}", d.product),
                    "product_name": d.product_name,
                    "hidraw_nodes": d.hidraw_nodes,
                    "enabled": enabled,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if devices.is_empty() {
        println!("No Keychron devices found.");
        return Ok(());
    }
    for d in &devices {
        let enabled = d
            .hidraw_nodes
            .first()
            .and_then(|n| node_is_enabled(n).ok())
            .unwrap_or(false);
        println!(
            "{}  {}  [{}]",
            d.id,
            d.product_name.as_deref().unwrap_or("Keychron device"),
            if enabled { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}

fn cmd_set_state(id: &str, enabled: bool) -> std::io::Result<()> {
    require_setup_done()?;
    let state_path = Path::new(STATE_FILE);
    let mut state = StateFile::load(state_path);
    state.set(id, enabled);
    state.save(state_path)?;

    // Apply immediately to any currently-attached nodes for this id, no replug needed.
    let devices = group_devices(scan_hidraw()?);
    if let Some(device) = devices.iter().find(|d| d.id == id) {
        for node in &device.hidraw_nodes {
            apply_permission(Path::new(node), enabled)?;
        }
    }
    Ok(())
}

fn cmd_udev_apply(devnode: &str) -> std::io::Result<()> {
    let devices = group_devices(scan_hidraw()?);
    let Some(device) = devices.iter().find(|d| d.hidraw_nodes.iter().any(|n| n == devnode)) else {
        return Ok(()); // not a Keychron device (or already gone), nothing to do
    };
    let state = StateFile::load(Path::new(STATE_FILE));
    apply_permission(Path::new(devnode), state.is_enabled(&device.id))
}
