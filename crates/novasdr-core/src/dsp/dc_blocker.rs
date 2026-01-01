pub struct MovingAverage {
    buf: std::collections::VecDeque<f32>,
    sum: f32,
    len: usize,
}

impl MovingAverage {
    pub fn new(len: usize) -> Self {
        Self {
            buf: std::collections::VecDeque::from(vec![0.0; len]),
            sum: 0.0,
            len,
        }
    }

    pub fn insert(&mut self, v: f32) -> f32 {
        let tail = self.buf.pop_back().unwrap_or(0.0);
        self.sum -= tail;
        self.buf.push_front(v);
        self.sum += v;
        self.sum / (self.len as f32)
    }

    pub fn get(&self) -> f32 {
        self.sum / (self.len as f32)
    }

    pub fn buf(&self) -> &std::collections::VecDeque<f32> {
        &self.buf
    }

    pub fn reset(&mut self) {
        self.sum = 0.0;
        self.buf.clear();
        self.buf.resize(self.len, 0.0);
    }
}

pub struct DcBlocker {
    delay: usize,
    ma1: MovingAverage,
    ma2: MovingAverage,
}

impl DcBlocker {
    pub fn new(delay: usize) -> Self {
        Self {
            delay,
            ma1: MovingAverage::new(delay),
            ma2: MovingAverage::new(delay),
        }
    }

    pub fn remove_dc(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            let ma1 = self.ma1.insert(*s);
            let ma2 = self.ma2.insert(ma1);
            let delayed = *self.ma1.buf().get(self.delay - 1).unwrap_or(&0.0);
            *s = delayed - ma2;
        }
    }

    pub fn reset(&mut self) {
        self.ma1.reset();
        self.ma2.reset();
    }
}
