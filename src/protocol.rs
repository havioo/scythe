pub fn query(command: u8, route: u8) -> [u8; 64] {
    request(command, route, &[])
}

pub fn request(command: u8, route: u8, payload: &[u8]) -> [u8; 64] {
    assert!(payload.len() <= 49);
    let mut b = [0u8; 64];
    b[0] = 2;
    b[2] = 0x60;
    b[6] = 4 + payload.len() as u8;
    b[9] = route;
    b[10] = command;
    b[12] = payload.len() as u8;
    b[13..13 + payload.len()].copy_from_slice(payload);
    b[62] = b[..62].iter().fold(0, |a, x| a ^ x);
    b
}

pub fn valid(b: &[u8]) -> bool {
    b.len() == 64
        && b[0] == 2
        && b[2] == 0x60
        && b[..62].iter().fold(0, |a, x| a ^ x) == b[62]
        && b[12] <= 49
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_real_packets() {
        let capture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/headset-reports.json")).unwrap();
        let mut count = 0;
        for message in capture["normalized_result"]["messages"].as_array().unwrap() {
            let Some(bytes) = message["payload"]["bytes"].as_array() else {
                continue;
            };
            let b: Vec<u8> = bytes.iter().map(|v| v.as_u64().unwrap() as u8).collect();
            assert!(valid(&b));
            count += 1;
            if message["payload"]["kind"] == "WriteFile" && b[12] == 0 {
                assert_eq!(query(b[10], b[9]).as_slice(), b);
            }
        }
        assert_eq!(count, 249);
    }
    #[test]
    fn reject_corruption_and_truncation() {
        let mut b = query(0x21, 0x80);
        assert!(valid(&b));
        b[13] ^= 1;
        assert!(!valid(&b));
        assert!(!valid(&b[..63]));
    }
    #[test]
    fn replay_updated_firmware_mic_writes() {
        let capture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/microphone-reports.json"))
                .unwrap();
        let mut count = 0;
        for message in capture["normalized_result"]["messages"].as_array().unwrap() {
            if message["payload"]["kind"] != "WriteFile" {
                continue;
            }
            let Some(bytes) = message["payload"]["bytes"].as_array() else {
                continue;
            };
            let b: Vec<u8> = bytes.iter().map(|v| v.as_u64().unwrap() as u8).collect();
            if matches!(b[10], 0x96 | 0x98 | 0x99) {
                assert_eq!(
                    request(b[10], b[9], &b[13..13 + b[12] as usize]).as_slice(),
                    b
                );
                count += 1;
            }
        }
        assert!(count >= 8);
    }
}
