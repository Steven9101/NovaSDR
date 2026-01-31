use num_complex::Complex32;
use rand::Rng;

use novasdr_core::config::{Accelerator, AudioCompression};
use novasdr_core::dsp::demod::DemodulationMode;
use novasdr_core::dsp::fft::{FftEngine, FftSettings};

use crate::cli::BenchmarkKind;
use crate::state::{AgcSpeed, AudioParams};
use crate::ws::audio::AudioPipeline;

fn generate_random_vector_complex<T: Rng>(rng: &mut T, size: usize) -> Vec<Complex32> {
    let mut res: Vec<Complex32> = vec![Complex32::new(0.0, 0.0); size];
    let mut gen_rng = || rng.gen_range(-1.0..1.0);
    for v in res.iter_mut() {
        *v = Complex32::new(gen_rng(), gen_rng());
    }
    res
}

fn generate_random_vector_real<T: Rng>(rng: &mut T, size: usize) -> Vec<f32> {
    let mut res: Vec<f32> = vec![0.0; size];
    for v in res.iter_mut() {
        *v = rng.gen_range(-1.0..1.0);
    }
    res
}

fn ssb_benchmark(iterations: usize) -> anyhow::Result<()> {
    println!("Run ssb_benchmark for: iterations={} ...", iterations);

    let sample_rate = 12000;
    let audio_fft_size = 8192;
    let is_real_input = false;
    let compression = AudioCompression::Adpcm;
    let mut pipeline = AudioPipeline::new(sample_rate, audio_fft_size, compression)?;

    let mut rng = rand::thread_rng();
    let spectrum = generate_random_vector_complex(&mut rng, audio_fft_size);
    let params = AudioParams {
        l: 200,
        m: 400.0,
        r: 2000,
        mute: false,
        squelch_enabled: false,
        demodulation: DemodulationMode::Usb,
        agc_speed: AgcSpeed::Off,
        agc_attack_ms: None,
        agc_release_ms: None,
    };

    for idx in 0..iterations {
        let frame_num = idx as u64;
        let audio_mid_idx = params.m.floor() as i32;

        let _ = pipeline.process(&spectrum, frame_num, &params, is_real_input, audio_mid_idx)?;
    }
    Ok(())
}

fn fft_benchmark(
    accelerator: Accelerator,
    is_real: bool,
    iterations: usize,
    fft_size: usize,
) -> anyhow::Result<()> {
    println!(
        "Run fft_benchmark for: iterations={} fft_size={} is_real={} accelerator={:?} ...",
        iterations, fft_size, is_real, accelerator
    );
    let brightness_offset = 0;
    let downsample_levels = 8;
    let include_waterfall = true;
    let audio_max_fft_size = 8192;

    let settings = FftSettings {
        fft_size,
        is_real,
        brightness_offset,
        downsample_levels,
        audio_max_fft_size,
        accelerator,
    };
    let mut fft = FftEngine::new(settings)?;

    let half_size = fft_size / 2;
    let mut rng = rand::thread_rng();
    fft.load_complex_half_a(&generate_random_vector_complex(&mut rng, half_size));
    fft.load_complex_half_b(&generate_random_vector_complex(&mut rng, half_size));
    fft.load_real_half_a(&generate_random_vector_real(&mut rng, half_size));
    fft.load_real_half_b(&generate_random_vector_real(&mut rng, half_size));

    for _idx in 0..iterations {
        let _ = fft.execute(include_waterfall)?;
    }

    Ok(())
}

pub fn run_benchmark(
    kind: BenchmarkKind,
    iterations: Option<usize>,
    fftsize: Option<usize>,
) -> anyhow::Result<()> {
    let (accelerator, is_real) = match kind {
        BenchmarkKind::CpuFftComplex => (Accelerator::None, false),
        BenchmarkKind::CpuFftReal => (Accelerator::None, true),
        BenchmarkKind::ClFftComplex => (Accelerator::Clfft, false),
        BenchmarkKind::ClFftReal => (Accelerator::Clfft, true),
        BenchmarkKind::VkFftComplex => (Accelerator::Vkfft, false),
        BenchmarkKind::VkFftReal => (Accelerator::Vkfft, true),
        BenchmarkKind::Ssb => return ssb_benchmark(iterations.unwrap_or(500)),
    };

    fft_benchmark(
        accelerator,
        is_real,
        iterations.unwrap_or(100),
        fftsize.unwrap_or(131072),
    )
}
