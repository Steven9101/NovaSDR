use novasdr_core::config::{
    AudioCompression, Config, InputDriver, Limits, ReceiverConfig, ReceiverDefaults, ReceiverInput,
    SampleFormat, Server, SignalType, WaterfallCompression, WebSdr,
};

fn base_config(signal: SignalType) -> Config {
    let receiver = ReceiverConfig {
        id: "rx0".to_string(),
        name: "rx0".to_string(),
        input: ReceiverInput {
            sps: 2_000_000,
            frequency: 7_100_000,
            signal,
            fft_size: 131_072,
            brightness_offset: 0,
            audio_sps: 12_000,
            waterfall_size: 1024,
            waterfall_compression: WaterfallCompression::Zstd,
            audio_compression: AudioCompression::Flac,
            smeter_offset: 0,
            accelerator: novasdr_core::config::Accelerator::None,
            driver: InputDriver::Stdin {
                format: SampleFormat::S16,
            },
            defaults: ReceiverDefaults {
                frequency: -1,
                modulation: "USB".to_string(),
                ssb_lowcut_hz: None,
                ssb_highcut_hz: None,
                squelch_enabled: false,
            },
        },
    };
    Config {
        server: Server::default(),
        websdr: WebSdr::default(),
        limits: Limits::default(),
        receivers: vec![receiver],
        active_receiver_id: "rx0".to_string(),
    }
}

#[test]
fn runtime_iq_basefreq_and_defaults() {
    let cfg = base_config(SignalType::Iq);
    let rt = cfg.runtime().unwrap();
    assert!(!rt.is_real);
    assert_eq!(rt.fft_result_size, 131_072);
    assert_eq!(rt.basefreq, 6_100_000);
    assert_eq!(rt.total_bandwidth, 2_000_000);
    assert_eq!(rt.default_frequency, 7_100_000);
    assert!((rt.default_m - 65_536.0).abs() < 1e-6);
}

#[test]
fn runtime_real_basefreq_and_defaults() {
    let cfg = base_config(SignalType::Real);
    let rt = cfg.runtime().unwrap();
    assert!(rt.is_real);
    assert_eq!(rt.fft_result_size, 65_536);
    assert_eq!(rt.basefreq, 7_100_000);
    assert_eq!(rt.total_bandwidth, 1_000_000);
}
