/// Native GDI COLORREF (0x00BBGGRR). Unknown readings never imply a battery tier.
pub fn color(percent: Option<u8>) -> u32 {
    match percent {
        Some(60..=100) => 0xc2ef93,
        Some(20..=59) => 0x70d9f6,
        Some(0..=19) => 0x5b54f2,
        _ => 0xbac2b6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_user_thresholds() {
        for p in [60, 61, 99, 100] {
            assert_eq!(color(Some(p)), 0xc2ef93);
        }
        for p in [20, 21, 58, 59] {
            assert_eq!(color(Some(p)), 0x70d9f6);
        }
        for p in [0, 1, 18, 19] {
            assert_eq!(color(Some(p)), 0x5b54f2);
        }
        assert_eq!(color(None), 0xbac2b6);
        assert_eq!(color(Some(101)), 0xbac2b6);
    }
}
