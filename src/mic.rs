use crate::{State, protocol};

pub fn notification(report: &[u8]) -> Option<bool> {
    if protocol::valid(report)
        && report[1] == 2
        && report[10] == 0x55
        && report[11] == 2
        && report[12] == 1
    {
        match report[13] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    } else {
        None
    }
}
pub fn update(state: &mut State, muted: bool) {
    if state.hardware_mic_muted == Some(false) && muted && state.notifications_enabled {
        state.mic_mute_notifications = state.mic_mute_notifications.wrapping_add(1);
    }
    state.hardware_mic_muted = Some(muted);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_physical_mute_events() {
        let c: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/microphone-reports.json"))
                .unwrap();
        let mut events = Vec::new();
        for m in c["normalized_result"]["messages"].as_array().unwrap() {
            if let Some(bytes) = m["payload"]["bytes"].as_array() {
                let b: Vec<u8> = bytes.iter().map(|v| v.as_u64().unwrap() as u8).collect();
                if let Some(mute) = notification(&b) {
                    events.push(mute);
                }
            }
        }
        assert!(events.windows(2).any(|p| p == [true, false]));
    }
    #[test]
    fn no_startup_duplicate_or_unmute_toasts() {
        let mut s = State {
            notifications_enabled: true,
            ..Default::default()
        };
        update(&mut s, true);
        assert_eq!(s.mic_mute_notifications, 0);
        update(&mut s, false);
        update(&mut s, true);
        assert_eq!(s.mic_mute_notifications, 1);
        update(&mut s, true);
        assert_eq!(s.mic_mute_notifications, 1);
        s.notifications_enabled = false;
        update(&mut s, false);
        update(&mut s, true);
        assert_eq!(s.mic_mute_notifications, 1);
    }
}
