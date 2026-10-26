use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub theme: u8,
    #[serde(default)]
    pub mic_notifications: bool,
}
fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Scythe")
        .join("settings.json")
}
pub fn load() -> Settings {
    let current = path();
    let legacy = current
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("BlackSharkLocal").join("settings.json"));
    std::fs::read(&current)
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                legacy.as_ref().map_or(Err(error), std::fs::read)
            } else {
                Err(error)
            }
        })
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default()
}
pub fn save(value: &Settings) -> Result<(), String> {
    let p = path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = p.with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_preferences_keep_notifications_when_theme_is_added() {
        let mut settings: Settings = serde_json::from_str(r#"{"mic_notifications":true}"#).unwrap();
        assert!(settings.mic_notifications);
        assert_eq!(settings.theme, 0);
        settings.theme = 3;
        let saved = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&saved).unwrap();
        assert!(restored.mic_notifications);
        assert_eq!(restored.theme, 3);
        assert!(!restored.reduce_motion);
    }
}
