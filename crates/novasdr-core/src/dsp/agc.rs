use std::collections::VecDeque;

pub struct Agc {
    desired_level: f32,
    attack_coeff: f32,
    release_coeff: f32,
    look_ahead_samples: usize,

    enabled: bool,
    gain: f32,
    max_gain: f32,

    ring: Vec<f32>,
    ring_pos: usize,
    filled: usize,

    max_queue: VecDeque<(usize, f32)>,
    sample_index: usize,

    hang_time: usize,
    hang_counter: usize,
    hang_threshold: f32,
}

impl Agc {
    pub fn new(
        desired_level: f32,
        attack_ms: f32,
        release_ms: f32,
        lookahead_ms: f32,
        sample_rate: f32,
    ) -> Self {
        let look_ahead_samples = (lookahead_ms * sample_rate / 1000.0).round().max(1.0) as usize;

        let attack_coeff = 1.0 - (-1.0 / (attack_ms * 0.001 * sample_rate)).exp();
        let release_coeff = 1.0 - (-1.0 / (release_ms * 0.001 * sample_rate)).exp();

        let ring = vec![0.0; look_ahead_samples];

        Self {
            desired_level,
            attack_coeff,
            release_coeff,
            look_ahead_samples,

            enabled: true,
            gain: 1.0,
            max_gain: 10.0,

            ring,
            ring_pos: 0,
            filled: 0,

            max_queue: VecDeque::new(),
            sample_index: 0,

            hang_time: (0.05 * sample_rate).round().max(1.0) as usize,
            hang_counter: 0,
            hang_threshold: 0.05,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.reset();
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_attack_coeff(&mut self, coeff: f32) {
        self.attack_coeff = coeff;
    }

    pub fn set_release_coeff(&mut self, coeff: f32) {
        self.release_coeff = coeff;
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.filled = 0;
        self.max_queue.clear();
        self.sample_index = 0;
        self.hang_counter = 0;
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }

        for s in samples.iter_mut() {
            let input = *s;

            let idx = self.sample_index;
            self.sample_index = self.sample_index.wrapping_add(1);

            self.push_sample(idx, input);

            if self.filled < self.look_ahead_samples {
                self.filled += 1;
                if self.filled < self.look_ahead_samples {
                    *s = 0.0;
                    continue;
                }
            }

            let delayed = self.ring[self.ring_pos];
            let peak = self.current_peak();
            self.update_gain(peak);
            *s = delayed * self.gain;
        }
    }

    fn push_sample(&mut self, idx: usize, sample: f32) {
        let abs = sample.abs();

        while let Some((_, back_abs)) = self.max_queue.back().copied() {
            if back_abs <= abs {
                self.max_queue.pop_back();
            } else {
                break;
            }
        }
        self.max_queue.push_back((idx, abs));

        // Maintain a max over the last `look_ahead_samples` values.
        let window = self.look_ahead_samples;
        while let Some((front_idx, _)) = self.max_queue.front().copied() {
            if front_idx + window <= idx {
                self.max_queue.pop_front();
            } else {
                break;
            }
        }

        self.ring[self.ring_pos] = sample;
        self.ring_pos += 1;
        if self.ring_pos >= self.ring.len() {
            self.ring_pos = 0;
        }
    }

    fn current_peak(&self) -> f32 {
        self.max_queue.front().map(|(_, abs)| *abs).unwrap_or(0.0)
    }

    fn update_gain(&mut self, peak: f32) {
        let peak = peak.max(1e-12);

        if peak >= self.hang_threshold {
            self.hang_counter = self.hang_time;
        } else if self.hang_counter > 0 {
            self.hang_counter -= 1;
        }

        let target = (self.desired_level / peak).min(self.max_gain);

        if target <= self.gain {
            // Attack must be effectively immediate to prevent brief startup/edge bursts.
            // Release remains smoothed.
            self.gain = target;
            return;
        }

        if self.hang_counter > 0 {
            return;
        }

        self.gain = self.gain + (target - self.gain) * self.release_coeff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_tone_converges_to_desired_peak() {
        let sample_rate = 12_000.0;
        let mut agc = Agc::new(0.1, 100.0, 30.0, 100.0, sample_rate);

        let amp = 0.02;
        let freq = 1000.0;
        let secs = 2.0;
        let n = (secs * sample_rate) as usize;

        let mut buf = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / sample_rate;
            buf.push((2.0 * std::f32::consts::PI * freq * t).sin() * amp);
        }

        agc.process(&mut buf);

        let tail = &buf[(n * 3 / 4)..];
        let peak = tail.iter().copied().map(f32::abs).fold(0.0, f32::max);

        assert!(
            (peak - 0.1).abs() < 0.01,
            "expected peak near desired_level=0.1, got {peak}"
        );
    }

    #[test]
    fn startup_does_not_overshoot_badly() {
        let sample_rate = 12_000.0;
        let lookahead_ms = 100.0;
        let lookahead = ((lookahead_ms * sample_rate / 1000.0_f32)
            .round()
            .max(1.0_f32)) as usize;

        let mut agc = Agc::new(0.1, 100.0, 30.0, lookahead_ms, sample_rate);

        let amp = 0.02;
        let freq = 1000.0;
        let n = (0.5 * sample_rate) as usize;

        let mut buf = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / sample_rate;
            buf.push((2.0 * std::f32::consts::PI * freq * t).sin() * amp);
        }

        agc.process(&mut buf);

        // Output starts when the lookahead buffer becomes full.
        let start = lookahead.saturating_sub(1);
        let end = (start + 512).min(buf.len());
        let peak = buf[start..end]
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0, f32::max);

        assert!(
            peak <= 0.2,
            "expected no loud startup burst, got peak {peak}"
        );
    }

    #[test]
    fn max_gain_matches_previous_effective_cap() {
        let sample_rate = 48_000.0;
        let mut agc = Agc::new(0.1, 100.0, 30.0, 1.0, sample_rate);

        let mut buf = vec![1e-6; 4096];
        agc.process(&mut buf);

        let peak = buf.iter().copied().map(f32::abs).fold(0.0, f32::max);
        assert!(
            peak <= 1e-5 * 1.2,
            "expected gain capped near 10x, got peak {peak}"
        );
    }

    #[test]
    fn disabled_agc_is_passthrough() {
        let sample_rate = 12_000.0;
        let mut agc = Agc::new(0.1, 100.0, 30.0, 10.0, sample_rate);
        agc.set_enabled(false);

        let mut buf = vec![0.25, -0.5, 0.75];
        agc.process(&mut buf);
        assert_eq!(buf, vec![0.25, -0.5, 0.75]);
    }
}
