use crate::protocol;

/// Firmware balance attenuates one channel; center preserves both at full level.
pub fn levels(index: u8) -> (u8, u8) {
    let index = index.min(20);
    if index <= 10 {
        (100, index * 10)
    } else {
        ((20 - index) * 10, 100)
    }
}

pub fn notification(report: &[u8]) -> Option<u8> {
    if protocol::valid(report)
        && report[1] == 2
        && report[10] == 0x5c
        && report[11] == 2
        && report[12] == 1
    {
        Some(report[13]).filter(|v| *v <= 20)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_balance_endpoints_and_center() {
        assert_eq!(levels(0), (100, 0));
        assert_eq!(levels(5), (100, 50));
        assert_eq!(levels(10), (100, 100));
        assert_eq!(levels(15), (50, 100));
        assert_eq!(levels(20), (0, 100));
    }
    #[test]
    fn real_game_chat_packets_match_frame_builder() {
        let capture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/headset-reports.json")).unwrap();
        let mut values = Vec::new();
        for message in capture["normalized_result"]["messages"].as_array().unwrap() {
            if message["payload"]["kind"] != "WriteFile" {
                continue;
            }
            let Some(bytes) = message["payload"]["bytes"].as_array() else {
                continue;
            };
            let b: Vec<u8> = bytes.iter().map(|v| v.as_u64().unwrap() as u8).collect();
            if b[10] == 0xdc {
                assert_eq!(b[12], 1);
                assert_eq!(protocol::request(0xdc, 0x80, &[b[13]]).as_slice(), b);
                values.push(b[13]);
            }
        }
        for raw in [0, 10, 20] {
            assert!(values.contains(&raw));
        }
    }
}
