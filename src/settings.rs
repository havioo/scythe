use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
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
