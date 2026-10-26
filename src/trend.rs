use std::collections::VecDeque;

#[derive(Default)]
pub struct Trend {
    samples: VecDeque<(u64, u8)>,
}
impl Trend {
    pub fn sample(&mut self, time: u64, percent: u8, charging: Option<bool>) -> Option<u32> {
        if charging != Some(false) {
            self.samples.clear();
            return None;
        }
        if self.samples.back().is_some_and(|&(_, last)| percent > last) {
            self.samples.clear();
        }
        if self
            .samples
            .back()
            .is_none_or(|&(t, _)| time.saturating_sub(t) >= 30)
        {
            self.samples.push_back((time, percent));
        }
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| time.saturating_sub(t) > 7200)
        {
            self.samples.pop_front();
        }
        let &(start, first) = self.samples.front()?;
        if time.saturating_sub(start) < 1200
            || first.saturating_sub(percent) < 3
            || self.samples.len() < 3
        {
            return None;
        }
        // Least-squares slope over the session window; estimates are intentionally coarse.
        let n = self.samples.len() as f64;
        let mean_x = self
            .samples
            .iter()
            .map(|&(t, _)| (t - start) as f64)
            .sum::<f64>()
            / n;
        let mean_y = self.samples.iter().map(|&(_, p)| p as f64).sum::<f64>() / n;
        let numerator = self
            .samples
            .iter()
            .map(|&(t, p)| ((t - start) as f64 - mean_x) * (p as f64 - mean_y))
            .sum::<f64>();
        let denominator = self
            .samples
            .iter()
            .map(|&(t, _)| ((t - start) as f64 - mean_x).powi(2))
            .sum::<f64>();
        let slope = numerator / denominator;
        if !slope.is_finite() || slope >= 0. {
            return None;
        }
        let minutes = percent as f64 / -slope / 60.;
        if minutes > 10080. {
            return None;
        }
        Some(((minutes / 5.).round() * 5.) as u32)
    }
    pub fn reset(&mut self) {
        self.samples.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_real_discharge() {
        let mut t = Trend::default();
        assert_eq!(t.sample(0, 80, Some(false)), None);
        assert_eq!(t.sample(600, 78, Some(false)), None);
        assert_eq!(t.sample(1200, 76, Some(false)), Some(380));
        assert_eq!(t.sample(1230, 77, Some(false)), None);
        assert_eq!(t.sample(1260, 78, Some(true)), None);
    }
    #[test]
    fn unknown_charge_and_flat_battery_never_estimate() {
        let mut t = Trend::default();
        for time in (0..3600).step_by(30) {
            assert_eq!(t.sample(time, 50, Some(false)), None);
        }
        assert_eq!(t.sample(3600, 40, None), None);
    }
}
