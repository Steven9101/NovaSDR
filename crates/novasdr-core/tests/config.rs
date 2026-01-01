#[test]
fn websdr_register_url_default_is_present() {
    let websdr = novasdr_core::config::WebSdr {
        register_online: true,
        ..novasdr_core::config::WebSdr::default()
    };

    let cfg = novasdr_core::config::Config {
        server: novasdr_core::config::Server::default(),
        websdr,
        limits: novasdr_core::config::Limits::default(),
        receivers: vec![novasdr_core::config::ReceiverConfig {
            id: "rx0".to_string(),
            name: "rx0".to_string(),
            input: novasdr_core::config::ReceiverInput {
                sps: 2_048_000,
                frequency: 100_900_000,
                signal: novasdr_core::config::SignalType::Iq,
                fft_size: 131_072,
                brightness_offset: 0,
                audio_sps: 12_000,
                waterfall_size: 1024,
                waterfall_compression: novasdr_core::config::WaterfallCompression::Zstd,
                audio_compression: novasdr_core::config::AudioCompression::Flac,
                smeter_offset: 0,
                accelerator: novasdr_core::config::Accelerator::None,
                driver: novasdr_core::config::InputDriver::Stdin {
                    format: novasdr_core::config::SampleFormat::U8,
                },
                defaults: novasdr_core::config::ReceiverDefaults::default(),
            },
        }],
        active_receiver_id: "rx0".to_string(),
    };

    assert!(cfg.websdr.register_online);
    assert_eq!(
        cfg.websdr.register_url,
        "https://sdr-list.xyz/api/update_websdr"
    );
}
