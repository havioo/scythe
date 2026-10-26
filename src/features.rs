use serde::Serialize;

pub const FREQUENCIES: [&str; 10] = [
    "31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];
pub const PRESETS: [(&str, [i8; 10]); 7] = [
    ("Flat", [0; 10]),
    ("eSports", [-4, -3, -2, 0, 1, 3, 4, 3, 1, 0]),
    ("FPS", [-5, -4, -2, -1, 1, 4, 5, 3, 0, -1]),
    ("Music", [3, 2, 1, 0, -1, 0, 1, 2, 3, 2]),
    ("Focus", [-3, -2, -1, 0, 1, 2, 3, 2, 1, 0]),
    ("Warm", [3, 3, 2, 1, 0, 0, -1, -1, 0, 1]),
    ("Cinema", [4, 3, 2, 0, -1, 0, 1, 2, 3, 2]),
];
#[derive(Clone, Debug, Default, Serialize)]
pub struct Features {
    pub bands: Option<[i8; 10]>,
    pub sleep: Option<u8>,
    pub anc: Option<[u8; 2]>,
    pub wireless: Option<u8>,
    pub voice_prompt: Option<bool>,
    pub led: Option<u8>,
}
pub fn encode_band(value: i8) -> Result<u8, String> {
    if !(-12..=12).contains(&value) {
        return Err("EQ bands must be between -12 and +12 dB".into());
    }
    Ok(if value < 0 {
        128 + value.unsigned_abs()
    } else {
        value as u8
    })
}
pub fn decode_bands(bytes: &[u8]) -> Option<[i8; 10]> {
    if bytes.len() != 10 {
        return None;
    }
    let mut bands = [0; 10];
    for (index, &byte) in bytes.iter().enumerate() {
        let value = if byte > 128 {
            128i16 - byte as i16
        } else {
            byte as i16
        };
        if !(-12..=12).contains(&value) {
            return None;
        }
        bands[index] = value as i8;
    }
    Some(bands)
}
pub fn decode_anc(bytes: &[u8]) -> Option<[u8; 2]> {
    if bytes.len() == 2 && matches!(bytes[0], 0 | 1 | 80) && (1..=4).contains(&bytes[1]) {
        Some([bytes[0], bytes[1]])
    } else {
        None
    }
}
pub fn decode_voice(bytes: &[u8]) -> Option<bool> {
    if bytes.len() == 2 && bytes[0] == 0 && bytes[1] <= 1 {
        Some(bytes[1] == 1)
    } else {
        None
    }
}
pub fn parse_minutes(value: &str) -> Result<u8, &'static str> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Enter whole minutes from 1 to 255.");
    }
    value
        .parse::<u8>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or("Enter whole minutes from 1 to 255.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sign_magnitude_matches_wire_examples() {
        assert_eq!(encode_band(-3).unwrap(), 131);
        assert_eq!(encode_band(4).unwrap(), 4);
        assert_eq!(
            decode_bands(&[140, 131, 129, 0, 1, 2, 3, 4, 5, 12]),
            Some([-12, -3, -1, 0, 1, 2, 3, 4, 5, 12])
        );
        assert!(encode_band(13).is_err());
        assert!(decode_bands(&[127; 10]).is_none());
    }
    #[test]
    fn real_feature_replies_decode_with_exact_domains() {
        let reports: Vec<Vec<u8>> =
            serde_json::from_str(include_str!("../tests/fixtures/feature-replies.json")).unwrap();
        let mut anc_modes = std::collections::BTreeSet::new();
        let mut voice_states = std::collections::BTreeSet::new();
        let mut eq_seen = false;
        for report in reports {
            assert!(crate::protocol::valid(&report));
            let payload = &report[13..13 + report[12] as usize];
            match report[10] {
                0x12 => {
                    anc_modes.insert(decode_anc(payload).unwrap()[0]);
                }
                0x65 => {
                    voice_states.insert(decode_voice(payload).unwrap());
                }
                0x15 => {
                    assert!(decode_bands(&payload[1..]).is_some());
                    eq_seen = true;
                }
                _ => {}
            }
        }
        assert_eq!(anc_modes, std::collections::BTreeSet::from([0, 1, 80]));
        assert_eq!(
            voice_states,
            std::collections::BTreeSet::from([false, true])
        );
        assert!(eq_seen);
        assert!(decode_anc(&[128, 1]).is_none());
        assert!(decode_anc(&[1, 0]).is_none());
        assert!(decode_voice(&[1, 1]).is_none());
        assert!(decode_voice(&[0, 255]).is_none());
    }
    #[test]
    fn custom_minutes_reject_invalid_units_and_overflow() {
        for value in ["", "0", "256", "1000", "1.5", "-1", "15m", " 15", "15 "] {
            assert!(parse_minutes(value).is_err(), "{value}");
        }
        for value in [1, 7, 37, 255] {
            assert_eq!(parse_minutes(&value.to_string()), Ok(value));
        }
    }
}
