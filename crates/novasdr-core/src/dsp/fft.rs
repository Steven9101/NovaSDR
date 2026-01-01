use crate::config::Accelerator;
use crate::dsp::window::hann_window;
use anyhow::Context;
use num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::{Fft as RustFft, FftPlanner};
#[cfg(feature = "clfft")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FftSettings {
    pub fft_size: usize,
    pub is_real: bool,
    pub brightness_offset: i32,
    pub downsample_levels: usize,
    pub audio_max_fft_size: usize,
    pub accelerator: Accelerator,
}

#[derive(Debug, Clone)]
pub struct FftResult {
    pub normalize: f32,
    pub quantized_concat: Option<Arc<[i8]>>,
    pub quantized_level_offsets: Option<Arc<[usize]>>,
}

pub struct FftEngine {
    settings: FftSettings,
    window: Vec<f32>,
    complex_fft: ComplexFft,
    real_fft: Arc<dyn RealToComplex<f32>>,
    real_scratch: Vec<Complex32>,
    real_spectrum_full: Vec<Complex32>,
    real_frame: Vec<f32>,
    #[cfg(feature = "clfft")]
    clfft_real: Option<crate::dsp::clfft::ClfftRealFft>,
    complex_frame: Vec<Complex32>,
    complex_half_a: Vec<Complex32>,
    complex_half_b: Vec<Complex32>,
    real_half_a: Vec<f32>,
    real_half_b: Vec<f32>,
}

enum ComplexFft {
    Cpu(Arc<dyn RustFft<f32>>),
    #[cfg(feature = "clfft")]
    Clfft(crate::dsp::clfft::ClfftComplexFft),
    #[cfg(feature = "vkfft")]
    Vkfft(crate::dsp::vkfft::VkfftComplexFft),
}

impl ComplexFft {
    fn process(&mut self, data: &mut [Complex32]) -> anyhow::Result<()> {
        match self {
            ComplexFft::Cpu(fft) => {
                fft.process(data);
                Ok(())
            }
            #[cfg(feature = "clfft")]
            ComplexFft::Clfft(fft) => fft.process_inplace(data),
            #[cfg(feature = "vkfft")]
            ComplexFft::Vkfft(fft) => fft.process_inplace(data),
        }
    }
}

impl FftEngine {
    pub fn new(settings: FftSettings) -> anyhow::Result<Self> {
        // FFTW/clFFT pipelines allow mixed-radix FFT sizes. RustFFT/RealFFT planners will reject
        // unsupported sizes if any; keep only the minimum-size guard here.
        anyhow::ensure!(settings.fft_size >= 8, "fft_size too small");
        anyhow::ensure!(
            settings.downsample_levels >= 1,
            "downsample_levels must be >= 1"
        );

        let fft_size = settings.fft_size;
        let window = hann_window(fft_size);

        let complex_fft = match settings.accelerator {
            Accelerator::None | Accelerator::Unsupported => {
                let mut complex_planner = FftPlanner::<f32>::new();
                ComplexFft::Cpu(complex_planner.plan_fft_forward(fft_size))
            }
            Accelerator::Clfft => {
                if settings.is_real {
                    let mut complex_planner = FftPlanner::<f32>::new();
                    ComplexFft::Cpu(complex_planner.plan_fft_forward(fft_size))
                } else {
                    #[cfg(feature = "clfft")]
                    {
                        ComplexFft::Clfft(crate::dsp::clfft::ClfftComplexFft::new(fft_size)?)
                    }
                    #[cfg(not(feature = "clfft"))]
                    {
                        anyhow::bail!(
                            "accelerator = \"clfft\" requires building with --features clfft"
                        );
                    }
                }
            }
            Accelerator::Vkfft => {
                if settings.is_real {
                    static WARNED: std::sync::Once = std::sync::Once::new();
                    WARNED.call_once(|| {
                        tracing::warn!(
                            "vkfft accelerator is not used for real input; falling back to CPU"
                        );
                    });
                    let mut complex_planner = FftPlanner::<f32>::new();
                    ComplexFft::Cpu(complex_planner.plan_fft_forward(fft_size))
                } else {
                    #[cfg(feature = "vkfft")]
                    {
                        ComplexFft::Vkfft(crate::dsp::vkfft::VkfftComplexFft::new(fft_size)?)
                    }
                    #[cfg(not(feature = "vkfft"))]
                    {
                        anyhow::bail!(
                            "accelerator = \"vkfft\" requires building with --features vkfft"
                        );
                    }
                }
            }
        };

        let mut real_planner = RealFftPlanner::<f32>::new();
        let real_fft = real_planner.plan_fft_forward(fft_size);

        let real_scratch = real_fft.make_scratch_vec();
        let real_spectrum_full = real_fft.make_output_vec();
        let real_frame = vec![0.0f32; fft_size];

        #[cfg(feature = "clfft")]
        let clfft_real = if settings.accelerator == Accelerator::Clfft && settings.is_real {
            Some(crate::dsp::clfft::ClfftRealFft::new(fft_size, &window)?)
        } else {
            None
        };
        #[cfg(not(feature = "clfft"))]
        if settings.accelerator == Accelerator::Clfft && settings.is_real {
            anyhow::bail!("accelerator = \"clfft\" requires building with --features clfft");
        }

        Ok(Self {
            settings,
            window,
            complex_fft,
            real_fft,
            real_scratch,
            real_spectrum_full,
            real_frame,
            #[cfg(feature = "clfft")]
            clfft_real,
            complex_frame: vec![Complex32::new(0.0, 0.0); fft_size],
            complex_half_a: vec![Complex32::new(0.0, 0.0); fft_size / 2],
            complex_half_b: vec![Complex32::new(0.0, 0.0); fft_size / 2],
            real_half_a: vec![0.0; fft_size / 2],
            real_half_b: vec![0.0; fft_size / 2],
        })
    }

    pub fn load_real_half_a(&mut self, half: &[f32]) {
        debug_assert_eq!(half.len(), self.settings.fft_size / 2);
        self.real_half_a.copy_from_slice(half);
    }

    pub fn load_real_half_b(&mut self, half: &[f32]) {
        debug_assert_eq!(half.len(), self.settings.fft_size / 2);
        self.real_half_b.copy_from_slice(half);
    }

    pub fn load_complex_half_a(&mut self, half: &[Complex32]) {
        debug_assert_eq!(half.len(), self.settings.fft_size / 2);
        self.complex_half_a.copy_from_slice(half);
    }

    pub fn load_complex_half_b(&mut self, half: &[Complex32]) {
        debug_assert_eq!(half.len(), self.settings.fft_size / 2);
        self.complex_half_b.copy_from_slice(half);
    }

    pub fn execute(&mut self, include_waterfall: bool) -> anyhow::Result<FftResult> {
        if self.settings.is_real {
            self.execute_real(include_waterfall)
        } else {
            self.execute_complex(include_waterfall)
        }
    }

    pub fn spectrum_for_audio(&self) -> &[Complex32] {
        if self.settings.is_real {
            let half = self.settings.fft_size / 2;
            &self.real_spectrum_full[..half]
        } else {
            &self.complex_frame
        }
    }

    fn execute_real(&mut self, include_waterfall: bool) -> anyhow::Result<FftResult> {
        let n = self.settings.fft_size;
        let half = n / 2;
        let fft_result_size = half;

        #[cfg(feature = "clfft")]
        let used_clfft = if let Some(clfft) = self.clfft_real.as_mut() {
            clfft.load_real_input(&self.real_half_a, &self.real_half_b)?;
            clfft.process_fft(&mut self.real_spectrum_full)?;
            true
        } else {
            false
        };
        #[cfg(not(feature = "clfft"))]
        let used_clfft = false;

        if !used_clfft {
            // Apply the window on CPU, then FFT.
            for i in 0..half {
                let a = self.real_half_a[i] * self.window[i];
                let b = self.real_half_b[i] * self.window[i + half];
                self.real_frame[i] = a;
                self.real_frame[i + half] = b;
            }
            self.real_fft
                .process_with_scratch(
                    &mut self.real_frame,
                    &mut self.real_spectrum_full,
                    &mut self.real_scratch,
                )
                .context("real fft")?;
        }

        // Normalize by N to keep the output scale consistent across FFT backends.
        let normalize = n as f32;
        let size_log2 = (n.ilog2() as i32) + self.settings.brightness_offset;

        let (quantized_concat, offsets) = if include_waterfall {
            #[cfg(feature = "clfft")]
            if used_clfft {
                if let Some(clfft) = self.clfft_real.as_mut() {
                    match clfft.quantize_and_downsample(
                        self.settings.downsample_levels,
                        size_log2,
                        normalize,
                    ) {
                        Ok((q, o)) => (Some(q.into()), Some(o.into())),
                        Err(e) => {
                            static WARNED: AtomicBool = AtomicBool::new(false);
                            if !WARNED.swap(true, Ordering::Relaxed) {
                                tracing::warn!(error = %e, "clFFT real waterfall quantization failed; dropping waterfall frame (audio prioritized)");
                            }
                            (None, None)
                        }
                    }
                } else {
                    let (q, o) = quantize_and_downsample_cpu(
                        &self.real_spectrum_full[..fft_result_size],
                        normalize,
                        0,
                        self.settings.downsample_levels,
                        size_log2,
                    );
                    (Some(q.into()), Some(o.into()))
                }
            } else {
                let (q, o) = quantize_and_downsample_cpu(
                    &self.real_spectrum_full[..fft_result_size],
                    normalize,
                    0,
                    self.settings.downsample_levels,
                    size_log2,
                );
                (Some(q.into()), Some(o.into()))
            }
            #[cfg(not(feature = "clfft"))]
            {
                let (q, o) = quantize_and_downsample_cpu(
                    &self.real_spectrum_full[..fft_result_size],
                    normalize,
                    0,
                    self.settings.downsample_levels,
                    size_log2,
                );
                (Some(q.into()), Some(o.into()))
            }
        } else {
            (None, None)
        };

        Ok(FftResult {
            normalize,
            quantized_concat,
            quantized_level_offsets: offsets,
        })
    }

    fn execute_complex(&mut self, include_waterfall: bool) -> anyhow::Result<FftResult> {
        let n = self.settings.fft_size;
        let half = n / 2;
        let normalize = n as f32;
        let size_log2 = (n.ilog2() as i32) + self.settings.brightness_offset;
        let base_idx = (n / 2) + 1;

        // Prefer GPU windowing + GPU waterfall quantization when using clFFT for complex input.
        // If kernels fail or produce invalid power, fall back to the CPU window/quantize path.
        #[cfg(feature = "clfft")]
        {
            if include_waterfall {
                if let ComplexFft::Clfft(fft) = &mut self.complex_fft {
                    // Assemble contiguous complex frame (unwindowed) for upload.
                    self.complex_frame[..half].copy_from_slice(&self.complex_half_a);
                    self.complex_frame[half..].copy_from_slice(&self.complex_half_b);

                    let gpu_res: anyhow::Result<FftResult> = (|| {
                        fft.window_and_process_inplace(&self.complex_frame)?;
                        let (q, o) = fft.quantize_and_downsample(
                            base_idx,
                            self.settings.downsample_levels,
                            size_log2,
                            normalize,
                        )?;

                        let max_p = fft.max_power()?;
                        if !max_p.is_finite() || max_p <= 1e-20 {
                            anyhow::bail!("clFFT produced invalid spectrum (max_power={max_p})");
                        }

                        fft.read_fft_output(&mut self.complex_frame)?;
                        Ok(FftResult {
                            normalize,
                            quantized_concat: Some(q.into()),
                            quantized_level_offsets: Some(o.into()),
                        })
                    })();

                    match gpu_res {
                        Ok(res) => return Ok(res),
                        Err(e) => {
                            static WARNED: AtomicBool = AtomicBool::new(false);
                            if !WARNED.swap(true, Ordering::Relaxed) {
                                tracing::warn!(error = %e, "clFFT complex GPU path failed; falling back to CPU");
                            }
                        }
                    }
                }
            }
        }

        // Prefer GPU windowing + GPU waterfall quantization when using VkFFT for complex input.
        // If kernels fail or produce invalid power, fall back to the CPU window/quantize path.
        #[cfg(feature = "vkfft")]
        {
            if include_waterfall {
                if let ComplexFft::Vkfft(fft) = &mut self.complex_fft {
                    self.complex_frame[..half].copy_from_slice(&self.complex_half_a);
                    self.complex_frame[half..].copy_from_slice(&self.complex_half_b);

                    let gpu_res: anyhow::Result<FftResult> = (|| {
                        fft.window_and_process_inplace(&self.complex_frame)?;
                        let (q, o) = fft.quantize_and_downsample(
                            base_idx,
                            self.settings.downsample_levels,
                            size_log2,
                            normalize,
                        )?;

                        let max_p = fft.max_power()?;
                        if !max_p.is_finite() || max_p <= 1e-20 {
                            anyhow::bail!("VkFFT produced invalid spectrum (max_power={max_p})");
                        }

                        fft.read_fft_output(&mut self.complex_frame)?;
                        Ok(FftResult {
                            normalize,
                            quantized_concat: Some(q.into()),
                            quantized_level_offsets: Some(o.into()),
                        })
                    })();

                    match gpu_res {
                        Ok(res) => return Ok(res),
                        Err(e) => {
                            static WARNED: std::sync::atomic::AtomicBool =
                                std::sync::atomic::AtomicBool::new(false);
                            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                tracing::warn!(error = %e, "vkfft complex GPU path failed; falling back to CPU");
                            }
                        }
                    }
                }
            }
        }

        // CPU: apply window then FFT and CPU waterfall.
        for i in 0..half {
            self.complex_frame[i] = self.complex_half_a[i] * self.window[i];
            self.complex_frame[i + half] = self.complex_half_b[i] * self.window[i + half];
        }
        self.complex_fft.process(&mut self.complex_frame)?;

        let (quantized_concat, offsets) = if include_waterfall {
            let (q, o) = quantize_and_downsample_cpu(
                &self.complex_frame,
                normalize,
                base_idx,
                self.settings.downsample_levels,
                size_log2,
            );
            (Some(q.into()), Some(o.into()))
        } else {
            (None, None)
        };

        Ok(FftResult {
            normalize,
            quantized_concat,
            quantized_level_offsets: offsets,
        })
    }
}

pub fn quantize_and_downsample_cpu(
    spectrum: &[Complex32],
    normalize: f32,
    base_idx: usize,
    levels: usize,
    size_log2: i32,
) -> (Vec<i8>, Vec<usize>) {
    let n = spectrum.len();
    let mut power = vec![0.0f32; n];
    let mut quantized_base = vec![0i8; n];

    for i in 0..n {
        let src = (i + base_idx) % n;
        let v = spectrum[src] / normalize;
        let p = v.re.mul_add(v.re, v.im * v.im).max(0.0);
        power[i] = p;
        quantized_base[i] = quantize_power(p, size_log2);
    }

    let mut offsets = Vec::with_capacity(levels);
    offsets.push(0usize);
    let mut out = Vec::with_capacity(n * 2);
    out.extend_from_slice(&quantized_base);

    let mut cur_power = power;
    let mut cur_len = n;
    let mut cur_offset = n;

    for level in 1..levels {
        let next_len = cur_len / 2;
        offsets.push(cur_offset);
        let mut next_power = vec![0.0f32; next_len];
        let mut next_quant = vec![0i8; next_len];
        let power_offset = size_log2 - (level as i32) - 1;
        for i in 0..next_len {
            let p = cur_power[i * 2] + cur_power[i * 2 + 1];
            next_power[i] = p;
            next_quant[i] = quantize_power(p, power_offset);
        }
        out.extend_from_slice(&next_quant);
        cur_power = next_power;
        cur_len = next_len;
        cur_offset += next_len;
    }

    (out, offsets)
}

fn quantize_power(power: f32, power_offset: i32) -> i8 {
    let p = power.max(1e-30);
    let db = 20.0 * p.log10() + 127.0 + (power_offset as f32) * 6.020_6;
    db.clamp(-128.0, 127.0).round() as i8
}
