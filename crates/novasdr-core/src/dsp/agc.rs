pub struct Agc {
    desired_level: f32,
    attack_coeff: f32,
    release_coeff: f32,
    fast_attack_coeff: f32,
    am_attack_coeff: f32,
    am_release_coeff: f32,
    look_ahead_samples: usize,
    gains: Vec<f32>,
    lookahead: std::collections::VecDeque<f32>,
    lookahead_max: std::collections::VecDeque<f32>,
    max_gain: f32,
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
        let fast_attack_coeff = 1.0 - (-1.0 / (0.5 * 0.001 * sample_rate)).exp();
        let am_attack_coeff = attack_coeff * 0.1;
        let am_release_coeff = release_coeff * 0.1;

        Self {
            desired_level,
            attack_coeff,
            release_coeff,
            fast_attack_coeff,
            am_attack_coeff,
            am_release_coeff,
            look_ahead_samples,
            gains: vec![1.0; 5],
            lookahead: std::collections::VecDeque::new(),
            lookahead_max: std::collections::VecDeque::new(),
            max_gain: 1000.0,
            hang_time: (0.05 * sample_rate) as usize,
            hang_counter: 0,
            hang_threshold: 0.05,
        }
    }

    pub fn set_attack_coeff(&mut self, coeff: f32) {
        self.attack_coeff = coeff;
        self.am_attack_coeff = coeff * 0.1;
    }

    pub fn set_release_coeff(&mut self, coeff: f32) {
        self.release_coeff = coeff;
        self.am_release_coeff = coeff * 0.1;
    }

    pub fn reset(&mut self) {
        self.gains.fill(1.0);
        self.lookahead.clear();
        self.lookahead_max.clear();
        self.hang_counter = 0;
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            self.push(*s);
            if self.lookahead.len() == self.look_ahead_samples {
                let current_sample = *self.lookahead.front().unwrap_or(&0.0);
                let peak = self.max();
                let desired_gain =
                    ((self.desired_level / (peak + 1e-15)) * 100.0).min(self.max_gain);
                self.apply_progressive_agc(desired_gain);

                let mut total_gain = 1.0f32;
                for g in self.gains.iter() {
                    total_gain *= *g;
                }
                total_gain = total_gain.min(self.max_gain);
                *s = current_sample * (total_gain * 0.01);
            } else {
                *s = 0.0;
            }
        }
    }

    fn push(&mut self, sample: f32) {
        self.lookahead.push_back(sample);
        while let Some(back) = self.lookahead_max.back().copied() {
            if back.abs() < sample.abs() {
                self.lookahead_max.pop_back();
            } else {
                break;
            }
        }
        self.lookahead_max.push_back(sample);
        if self.lookahead.len() > self.look_ahead_samples {
            self.pop();
        }
    }

    fn pop(&mut self) {
        if let Some(sample) = self.lookahead.pop_front() {
            if self.lookahead_max.front().copied() == Some(sample) {
                self.lookahead_max.pop_front();
            }
        }
    }

    fn max(&self) -> f32 {
        self.lookahead_max.front().copied().unwrap_or(0.0).abs()
    }

    fn apply_progressive_agc(&mut self, desired_gain: f32) {
        let stages = self.gains.len() as f32;
        for g in self.gains.iter_mut() {
            let stage_desired = desired_gain.powf(1.0 / stages).min(self.max_gain);

            if stage_desired < *g * self.hang_threshold {
                self.hang_counter = self.hang_time;
            }

            if self.hang_counter > 0 {
                self.hang_counter -= 1;
                continue;
            }

            let fast_gain =
                *g * (1.0 - self.fast_attack_coeff) + stage_desired * self.fast_attack_coeff;
            let slow_gain = if stage_desired < *g {
                *g * (1.0 - self.am_attack_coeff) + stage_desired * self.am_attack_coeff
            } else {
                *g * (1.0 - self.am_release_coeff) + stage_desired * self.am_release_coeff
            };
            *g = fast_gain.min(slow_gain).min(self.max_gain);
        }

        if desired_gain > self.gains[0] {
            self.gains[0] = (self.gains[0] * (1.0 - self.release_coeff * 0.1)
                + desired_gain * self.release_coeff * 0.1)
                .min(self.max_gain);
        }
    }
}
