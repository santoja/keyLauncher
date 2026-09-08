use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const KEYCHRON_VID: u16 = 0x3434;

/// One hidraw node as read straight off sysfs/udev, before grouping.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RawHidrawRecord {
    pub devnode: String,
    pub vendor: u16,
    pub product: u16,
    pub serial: Option<String>,
    pub product_name: Option<String>,
    /// USB device syspath, used as a stable-enough fallback identity when no serial is present.
    pub devpath: String,
}

/// A physical Keychron keyboard, possibly exposing several hidraw nodes
/// (keyboard usage page + VIA/vendor config usage page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub vendor: u16,
    pub product: u16,
    pub product_name: Option<String>,
    pub hidraw_nodes: Vec<String>,
}

pub fn resolve_id(record: &RawHidrawRecord) -> String {
    match &record.serial {
        Some(serial) if !serial.is_empty() => serial.clone(),
        _ => {
            let mut hasher = DefaultHasher::new();
            record.devpath.hash(&mut hasher);
            format!(
                "noserial-{:04x}-{:04x}-{:x}",
                record.vendor,
                record.product,
                hasher.finish()
            )
        }
    }
}

pub fn group_devices(records: Vec<RawHidrawRecord>) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    for record in records {
        let id = resolve_id(&record);
        if let Some(existing) = devices.iter_mut().find(|d| d.id == id) {
            existing.hidraw_nodes.push(record.devnode);
        } else {
            devices.push(Device {
                id,
                vendor: record.vendor,
                product: record.product,
                product_name: record.product_name,
                hidraw_nodes: vec![record.devnode],
            });
        }
    }
    devices
}

/// True if the world/group bits on this node currently allow non-root access.
pub fn node_is_enabled(devnode: &str) -> std::io::Result<bool> {
    let meta = std::fs::metadata(devnode)?;
    Ok(meta.permissions().mode() & 0o060 != 0)
}

pub fn apply_permission(devnode: &Path, enabled: bool) -> std::io::Result<()> {
    let mode = if enabled { 0o660 } else { 0o600 };
    std::fs::set_permissions(devnode, std::fs::Permissions::from_mode(mode))
}

fn read_attr(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_string())
}

fn read_hex_attr(dir: &Path, name: &str) -> Option<u16> {
    read_attr(dir, name).and_then(|s| u16::from_str_radix(&s, 16).ok())
}

/// Walk up from a hidraw device's real sysfs path to the ancestor directory
/// that holds the USB device attributes (idVendor/idProduct/serial/product).
fn find_usb_device_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("idVendor").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Enumerate hidraw devices belonging to Keychron (VID 0x3434) by walking /sys.
/// Thin I/O wrapper, intentionally untested here (needs real hardware).
pub fn scan_hidraw() -> std::io::Result<Vec<RawHidrawRecord>> {
    let mut records = Vec::new();
    let class_dir = Path::new("/sys/class/hidraw");
    if !class_dir.exists() {
        return Ok(records);
    }

    for entry in std::fs::read_dir(class_dir)? {
        let entry = entry?;
        let hidraw_name = entry.file_name().to_string_lossy().to_string();
        let syspath = entry.path().canonicalize()?;
        let Some(usb_dir) = find_usb_device_dir(&syspath) else {
            continue;
        };
        let Some(vendor) = read_hex_attr(&usb_dir, "idVendor") else {
            continue;
        };
        if vendor != KEYCHRON_VID {
            continue;
        }
        let product = read_hex_attr(&usb_dir, "idProduct").unwrap_or(0);
        let serial = read_attr(&usb_dir, "serial");
        let product_name = read_attr(&usb_dir, "product");

        records.push(RawHidrawRecord {
            devnode: format!("/dev/{hidraw_name}"),
            vendor,
            product,
            serial,
            product_name,
            devpath: usb_dir.to_string_lossy().to_string(),
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hidraw: &str, serial: Option<&str>, devpath: &str) -> RawHidrawRecord {
        RawHidrawRecord {
            devnode: hidraw.to_string(),
            vendor: KEYCHRON_VID,
            product: 0x0123,
            serial: serial.map(|s| s.to_string()),
            product_name: Some("Keychron V6 Max".to_string()),
            devpath: devpath.to_string(),
        }
    }

    #[test]
    fn groups_composite_device_by_shared_serial() {
        let records = vec![
            record("/dev/hidraw0", Some("ABC123"), "1-1"),
            record("/dev/hidraw1", Some("ABC123"), "1-1"),
        ];
        let devices = group_devices(records);
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].hidraw_nodes,
            vec!["/dev/hidraw0".to_string(), "/dev/hidraw1".to_string()]
        );
        assert_eq!(devices[0].id, "ABC123");
    }

    #[test]
    fn separate_serials_stay_separate_devices() {
        let records = vec![
            record("/dev/hidraw0", Some("AAA"), "1-1"),
            record("/dev/hidraw1", Some("BBB"), "1-2"),
        ];
        let devices = group_devices(records);
        assert_eq!(devices.len(), 2);
    }

    #[test]
    fn missing_serial_falls_back_to_stable_id_from_devpath() {
        let a = resolve_id(&record("/dev/hidraw0", None, "1-1"));
        let b = resolve_id(&record("/dev/hidraw0", None, "1-1"));
        let c = resolve_id(&record("/dev/hidraw0", None, "1-2"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("noserial-3434-0123-"));
    }
}
