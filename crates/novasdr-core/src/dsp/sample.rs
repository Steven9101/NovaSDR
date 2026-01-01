use crate::config::SampleFormat;
use anyhow::Context;
use std::io::Read;

pub struct SampleReader<R> {
    reader: R,
    format: SampleFormat,
    scratch_u8: Vec<u8>,
    scratch_i16: Vec<i16>,
    scratch_u16: Vec<u16>,
    scratch_f64: Vec<f64>,
}

impl<R: Read> SampleReader<R> {
    pub fn new(reader: R, format: SampleFormat) -> Self {
        Self {
            reader,
            format,
            scratch_u8: Vec::new(),
            scratch_i16: Vec::new(),
            scratch_u16: Vec::new(),
            scratch_f64: Vec::new(),
        }
    }

    pub fn read_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        match self.format {
            SampleFormat::U8 => self.read_u8_as_f32(out),
            SampleFormat::S8 => self.read_i8_as_f32(out),
            SampleFormat::U16 => self.read_u16_as_f32(out),
            SampleFormat::S16 => self.read_i16_as_f32(out),
            SampleFormat::Cs16 => self.read_i16_as_f32(out),
            SampleFormat::F32 | SampleFormat::Cf32 => self.read_f32_raw(out),
            SampleFormat::F64 => self.read_f64_as_f32(out),
        }
    }

    fn read_u8_as_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        if self.scratch_u8.len() < out.len() {
            self.scratch_u8.resize(out.len(), 0u8);
        }
        let raw = &mut self.scratch_u8[..out.len()];
        self.reader.read_exact(raw).context("input sample read")?;

        // Fast path: precomputed LUT for u8 -> f32 mapping.
        // This avoids per-sample branches and endian work in the hot loop.
        static U8_TO_F32: [f32; 256] = {
            let mut lut = [0.0f32; 256];
            let mut i = 0usize;
            while i < 256 {
                let signed = ((i as u8) ^ 0x80) as i8;
                lut[i] = (signed as f32) / 128.0;
                i += 1;
            }
            lut
        };
        for (dst, src) in out.iter_mut().zip(raw.iter().copied()) {
            *dst = U8_TO_F32[src as usize];
        }
        Ok(())
    }

    fn read_i8_as_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        if self.scratch_u8.len() < out.len() {
            self.scratch_u8.resize(out.len(), 0u8);
        }
        let raw = &mut self.scratch_u8[..out.len()];
        self.reader.read_exact(raw).context("input sample read")?;

        for (dst, src) in out.iter_mut().zip(raw.iter().copied()) {
            *dst = (src as i8) as f32 / 128.0;
        }
        Ok(())
    }

    fn read_u16_as_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        if self.scratch_u16.len() < out.len() {
            self.scratch_u16.resize(out.len(), 0u16);
        }
        let raw_u16 = &mut self.scratch_u16[..out.len()];
        let raw_bytes: &mut [u8] = bytemuck::cast_slice_mut(raw_u16);
        self.reader
            .read_exact(raw_bytes)
            .context("input sample read")?;

        for (dst, src) in out.iter_mut().zip(raw_u16.iter().copied()) {
            let signed = (src ^ 0x8000) as i16;
            *dst = (signed as f32) / 32768.0;
        }
        Ok(())
    }

    fn read_i16_as_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        if self.scratch_i16.len() < out.len() {
            self.scratch_i16.resize(out.len(), 0i16);
        }
        let raw_i16 = &mut self.scratch_i16[..out.len()];
        let raw_bytes: &mut [u8] = bytemuck::cast_slice_mut(raw_i16);
        self.reader
            .read_exact(raw_bytes)
            .context("input sample read")?;

        for (dst, src) in out.iter_mut().zip(raw_i16.iter().copied()) {
            *dst = (src as f32) / 32768.0;
        }
        Ok(())
    }

    fn read_f32_raw(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        let raw: &mut [u8] = bytemuck::cast_slice_mut(out);
        self.reader.read_exact(raw).context("input sample read")?;
        Ok(())
    }

    fn read_f64_as_f32(&mut self, out: &mut [f32]) -> anyhow::Result<()> {
        if self.scratch_f64.len() < out.len() {
            self.scratch_f64.resize(out.len(), 0.0f64);
        }
        let raw_f64 = &mut self.scratch_f64[..out.len()];
        let raw_bytes: &mut [u8] = bytemuck::cast_slice_mut(raw_f64);
        self.reader
            .read_exact(raw_bytes)
            .context("input sample read")?;

        for (dst, src) in out.iter_mut().zip(raw_f64.iter().copied()) {
            *dst = src as f32;
        }
        Ok(())
    }
}
