use num_complex::Complex32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodulationMode {
    Usb,
    Lsb,
    Am,
    Sam,
    Fm,
}

impl DemodulationMode {
    pub fn from_str_upper(s: &str) -> Option<Self> {
        match s {
            "USB" => Some(Self::Usb),
            "LSB" => Some(Self::Lsb),
            "AM" => Some(Self::Am),
            "SAM" => Some(Self::Sam),
            "FM" | "FMC" | "NFM" | "NBFM" | "WBFM" => Some(Self::Fm),
            _ => None,
        }
    }
}

pub fn negate_f32(arr: &mut [f32]) {
    for v in arr.iter_mut() {
        *v = -*v;
    }
}

pub fn negate_complex(arr: &mut [Complex32]) {
    for v in arr.iter_mut() {
        *v = -*v;
    }
}

pub fn add_f32(a: &mut [f32], b: &[f32]) {
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x += *y;
    }
}

pub fn add_complex(a: &mut [Complex32], b: &[Complex32]) {
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x += *y;
    }
}

pub fn am_envelope(iq: &[Complex32], out: &mut [f32]) {
    for (dst, v) in out.iter_mut().zip(iq.iter()) {
        *dst = (v.re * v.re + v.im * v.im).sqrt();
    }
}

pub fn sam_demod(iq: &[Complex32], carrier: &[Complex32], out: &mut [f32]) {
    let eps = 1e-6f32;
    for ((dst, v), c) in out.iter_mut().zip(iq.iter()).zip(carrier.iter()) {
        let mag = (c.re * c.re + c.im * c.im).sqrt().max(eps);
        let unit = Complex32::new(c.re / mag, c.im / mag);
        *dst = (*v * unit.conj()).re;
    }
}

pub fn polar_discriminator_fm(iq: &[Complex32], mut prev: Complex32, out: &mut [f32]) -> Complex32 {
    for (dst, v) in out.iter_mut().zip(iq.iter()) {
        let d = *v * prev.conj();
        *dst = d.arg();
        prev = *v;
    }
    prev
}

pub fn float_to_i16_centered(samples: &[f32], out: &mut [i16], mult: f32) {
    for (dst, s) in out.iter_mut().zip(samples.iter()) {
        let v = (s * mult + 32768.5).floor() as i32 - 32768;
        *dst = v.clamp(-32768, 32767) as i16;
    }
}

pub fn float_to_i8_centered(samples: &[f32], out: &mut [i8], mult: f32) {
    for (dst, s) in out.iter_mut().zip(samples.iter()) {
        let v = (s * mult + 128.5).floor() as i32 - 128;
        *dst = v.clamp(-128, 127) as i8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_to_i8_centered_maps_expected_range() {
        let samples: [f32; 8] = [-1.0, -0.5, 0.0, 0.5, 0.999, 1.0, 2.0, -2.0];
        let mut out = [0i8; 8];
        float_to_i8_centered(&samples, &mut out, 128.0);
        assert_eq!(out, [-128, -64, 0, 64, 127, 127, 127, -128]);
    }

    #[test]
    fn demodulation_mode_accepts_wbfm_alias() {
        assert_eq!(
            DemodulationMode::from_str_upper("WBFM"),
            Some(DemodulationMode::Fm)
        );
        assert_eq!(
            DemodulationMode::from_str_upper("NFM"),
            Some(DemodulationMode::Fm)
        );
        assert_eq!(
            DemodulationMode::from_str_upper("NBFM"),
            Some(DemodulationMode::Fm)
        );
    }
}
