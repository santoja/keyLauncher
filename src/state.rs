use std::collections::HashMap;
use std::path::Path;

pub const STATE_FILE: &str = "/var/lib/keylauncher/state.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StateFile(HashMap<String, bool>);

impl StateFile {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }

    /// Unseen devices default to enabled, matching the underlying udev rule's default-allow.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.0.get(id).copied().unwrap_or(true)
    }

    pub fn set(&mut self, id: &str, enabled: bool) {
        self.0.insert(id.to_string(), enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_id_defaults_to_enabled() {
        let state = StateFile::default();
        assert!(state.is_enabled("some-id"));
    }

    #[test]
    fn set_then_is_enabled_round_trips() {
        let mut state = StateFile::default();
        state.set("abc", false);
        assert!(!state.is_enabled("abc"));
        state.set("abc", true);
        assert!(state.is_enabled("abc"));
    }
}
