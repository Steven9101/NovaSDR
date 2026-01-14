use std::collections::VecDeque;

pub struct Agc {
    desired_level: f32,
    attack_coeff: f32,
    release_coeff: f32,
    fast_attack_coeff: f32,
    am_attack_coeff: f32,
    am_release_coeff: f32,
    look_ahead_samples: usize,
    gains: Vec<f32>,
    stage_root: f32,
    ring: Vec<f32>,
    ring_pos: usize,
    filled: usize,
    max_queue: VecDeque<(usize, f32)>,
    sample_index: usize,
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
        let gains = vec![1.0; 5];
        let stage_root = 1.0 / gains.len() as f32;

        Self {
            desired_level,
            attack_coeff,
            release_coeff,
            fast_attack_coeff,
            am_attack_coeff,
            am_release_coeff,
            look_ahead_samples,
            gains,
            stage_root,
            ring: vec![0.0; look_ahead_samples],
            ring_pos: 0,
            filled: 0,
            max_queue: VecDeque::new(),
            sample_index: 0,
            max_gain: 1000.0,
            hang_time: (0.05 * sample_rate).round().max(1.0) as usize,
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
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.filled = 0;
        self.max_queue.clear();
        self.sample_index = 0;
        self.hang_counter = 0;
    }

    pub fn process(&mut self, samples: &mut [f32]) {
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
            let desired_gain = ((self.desired_level / (peak + 1e-15)) * 100.0).min(self.max_gain);
            self.apply_progressive_agc(desired_gain);

            let mut total_gain = 1.0f32;
            for g in self.gains.iter() {
                total_gain *= *g;
            }
            total_gain = total_gain.min(self.max_gain);
            *s = delayed * (total_gain * 0.01);
        }
    }

    fn push_sample(&mut self, idx: usize, sample: f32) {
        let abs = sample.abs();

        while let Some((_, back_abs)) = self.max_queue.back().copied() {
            if back_abs < abs {
                self.max_queue.pop_back();
            } else {
                break;
            }
        }
        self.max_queue.push_back((idx, abs));

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

    fn apply_progressive_agc(&mut self, desired_gain: f32) {
        let stage_desired = desired_gain.powf(self.stage_root).min(self.max_gain);

        for g in self.gains.iter_mut() {
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
