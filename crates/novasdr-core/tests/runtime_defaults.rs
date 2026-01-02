use novasdr_core::config::{
    Accelerator, AudioCompression, Config, InputDriver, Limits, ReceiverConfig, ReceiverDefaults,
    ReceiverInput, SampleFormat, Server, SignalType, WaterfallCompression, WebSdr,
};

#[test]
fn runtime_defaults_use_configured_modulation() {
    let receiver = ReceiverConfig {
        id: "rx0".to_string(),
        name: "rx0".to_string(),
        input: ReceiverInput {
            sps: 60_000_000,
            frequency: 60_000_000,
            signal: SignalType::Real,
            fft_size: 1_048_576,
            brightness_offset: 0,
            audio_sps: 12_000,
            waterfall_size: 1024,
            waterfall_compression: WaterfallCompression::Zstd,
            audio_compression: AudioCompression::Flac,
            smeter_offset: 0,
            accelerator: Accelerator::Clfft,
            driver: InputDriver::Stdin {
                format: SampleFormat::S16,
            },
            defaults: ReceiverDefaults {
                frequency: -1,
                modulation: "LSB".to_string(),
                ssb_lowcut_hz: None,
                ssb_highcut_hz: None,
            },
        },
    };
    let cfg = Config {
        server: Server::default(),
        websdr: WebSdr::default(),
        limits: Limits::default(),
        receivers: vec![receiver],
        active_receiver_id: "rx0".to_string(),
    };
    let rt = cfg.runtime().unwrap();

    assert_eq!(rt.default_mode_str, "LSB");
    assert!(rt.audio_max_sps > 0);
    assert!(rt.audio_max_fft_size >= 32);
    assert!(rt.default_l >= 0);
    assert!(rt.default_r >= rt.default_l);
    assert!(
        (rt.default_r - rt.default_l) as usize <= rt.audio_max_fft_size.min(rt.fft_result_size)
    );
}

#[test]
fn runtime_defaults_respect_configured_ssb_passband() {
    let receiver = ReceiverConfig {
        id: "rx0".to_string(),
        name: "rx0".to_string(),
        input: ReceiverInput {
            sps: 2_000_000,
            frequency: 7_100_000,
            signal: SignalType::Iq,
            fft_size: 131_072,
            brightness_offset: 0,
            audio_sps: 48_000,
            waterfall_size: 1024,
            waterfall_compression: WaterfallCompression::Zstd,
            audio_compression: AudioCompression::Flac,
            smeter_offset: 0,
            accelerator: Accelerator::None,
            driver: InputDriver::Stdin {
                format: SampleFormat::S16,
            },
            defaults: ReceiverDefaults {
                frequency: -1,
                modulation: "USB".to_string(),
                ssb_lowcut_hz: Some(100),
                ssb_highcut_hz: Some(2800),
            },
        },
    };
    let cfg = Config {
        server: Server::default(),
        websdr: WebSdr::default(),
        limits: Limits::default(),
        receivers: vec![receiver],
        active_receiver_id: "rx0".to_string(),
    };
    let rt = cfg.runtime().unwrap();

    let expected_width_bins =
        ((2800_i64 * (rt.fft_result_size as i64) / rt.sps) - (100_i64 * (rt.fft_result_size as i64) / rt.sps))
            as i32;
    assert_eq!(rt.default_mode_str, "USB");
    assert_eq!(rt.default_r - rt.default_l, expected_width_bins);
}
