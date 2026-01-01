use novasdr_core::config::SampleFormat;
use novasdr_core::dsp::sample::SampleReader;
use std::io::Cursor;

fn read_all(mut reader: SampleReader<Cursor<Vec<u8>>>, len: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; len];
    reader.read_f32(&mut out).unwrap();
    out
}

#[test]
fn sample_reader_u8_to_f32_matches_compat_mapping() {
    // Map unsigned 8-bit centered at 128 to [-1, +1) via xor 0x80.
    let input = vec![0u8, 128u8, 255u8];
    let reader = SampleReader::new(Cursor::new(input), SampleFormat::U8);
    let out = read_all(reader, 3);

    assert!((out[0] - (-1.0)).abs() < 1e-6);
    assert!((out[1] - 0.0).abs() < 1e-6);
    assert!((out[2] - (127.0 / 128.0)).abs() < 1e-6);
}

#[test]
fn sample_reader_s16_to_f32_scales_by_32768() {
    let samples: [i16; 3] = [-32768, 0, 32767];
    let mut input = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        input.extend_from_slice(&s.to_ne_bytes());
    }

    let reader = SampleReader::new(Cursor::new(input), SampleFormat::S16);
    let out = read_all(reader, 3);

    assert!((out[0] - (-1.0)).abs() < 1e-6);
    assert!((out[1] - 0.0).abs() < 1e-6);
    assert!((out[2] - (32767.0 / 32768.0)).abs() < 1e-6);
}

#[test]
fn sample_reader_f32_is_zero_copy_into_output() {
    let samples: [f32; 3] = [0.25, -0.5, 1.0];
    let mut input = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        input.extend_from_slice(&s.to_ne_bytes());
    }

    let reader = SampleReader::new(Cursor::new(input), SampleFormat::F32);
    let out = read_all(reader, 3);
    assert!((out[0] - 0.25).abs() < 1e-6);
    assert!((out[1] - (-0.5)).abs() < 1e-6);
    assert!((out[2] - 1.0).abs() < 1e-6);
}

#[test]
fn sample_reader_f64_is_converted_to_f32() {
    let samples: [f64; 2] = [0.5, -2.0];
    let mut input = Vec::with_capacity(samples.len() * 8);
    for s in samples {
        input.extend_from_slice(&s.to_ne_bytes());
    }

    let reader = SampleReader::new(Cursor::new(input), SampleFormat::F64);
    let out = read_all(reader, 2);
    assert!((out[0] - 0.5).abs() < 1e-6);
    assert!((out[1] - (-2.0)).abs() < 1e-6);
}
