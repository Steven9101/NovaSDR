use crate::state::{AppState, ReceiverState};
use anyhow::Context;
use novasdr_core::dsp::{
    fft::{FftEngine, FftSettings},
    sample::SampleReader,
};
use num_complex::Complex32;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tokio::sync::mpsc::error::TrySendError as TokioTrySendError;

const SAMPLE_BUFFER_POOL_DEPTH: usize = 512;

#[cfg(feature = "vkfft")]
use novasdr_core::dsp::vkfft::VkfftWaterfallQuantizer;

pub fn start(state: Arc<AppState>) -> anyhow::Result<()> {
    start_events_task(state.clone());

    let receiver_count = state.receivers.len().max(1);
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut reader_threads_budget = available.saturating_sub(receiver_count);
    let mut waterfall_threads_budget = reader_threads_budget.saturating_sub(receiver_count);
    tracing::info!(
        available_parallelism = available,
        receiver_count,
        reader_threads_budget,
        waterfall_threads_budget,
        "DSP threading policy"
    );
    let soapy_semaphore = Arc::new(Mutex::new(()));

    for rx in state.receivers.values() {
        let state = state.clone();
        let rx = rx.clone();
        let rx_id = rx.receiver.id.clone();
        let use_reader_thread = reader_threads_budget > 0;
        reader_threads_budget = reader_threads_budget.saturating_sub(1);
        let use_waterfall_thread = waterfall_threads_budget > 0;
        waterfall_threads_budget = waterfall_threads_budget.saturating_sub(1);
        let soapy_semaphore = soapy_semaphore.clone();
        thread::Builder::new()
            .name(format!("novasdr-dsp-{rx_id}"))
            .spawn(move || {
                tracing::info!(receiver_id = %rx_id, "DSP thread started");
                if let Err(e) = run_dsp_loop(
                    state,
                    rx,
                    use_reader_thread,
                    use_waterfall_thread,
                    soapy_semaphore,
                ) {
                    if crate::shutdown::is_shutdown_requested() || is_expected_input_termination(&e)
                    {
                        tracing::info!(receiver_id = %rx_id, error = ?e, "DSP loop terminated");
                    } else {
                        tracing::error!(receiver_id = %rx_id, error = ?e, "DSP loop terminated");
                    }
                }
            })?;
    }

    Ok(())
}

fn is_expected_input_termination(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(ioe) = cause.downcast_ref::<io::Error>() {
            return matches!(
                ioe.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::Interrupted
            );
        }
    }
    false
}

fn run_dsp_loop(
    state: Arc<AppState>,
    receiver: Arc<ReceiverState>,
    use_reader_thread: bool,
    use_waterfall_thread: bool,
    soapy_semaphore: Arc<Mutex<()>>,
) -> anyhow::Result<()> {
    let stop_requested = Arc::new(AtomicBool::new(false));
    let (input, input_name) =
        crate::input::open(&receiver.receiver, stop_requested.clone(), soapy_semaphore)?;
    let sample_format = receiver.receiver.input.driver.get_sample_format();
    tracing::info!(
        receiver_id = %receiver.receiver.id,
        input = input_name,
        format = ?sample_format,
        "input opened"
    );
    let mut reader = SampleReader::new(input, sample_format);

    let rt = receiver.rt.clone();
    let settings = FftSettings {
        fft_size: rt.fft_size,
        is_real: rt.is_real,
        brightness_offset: rt.brightness_offset,
        downsample_levels: rt.downsample_levels,
        audio_max_fft_size: rt.audio_max_fft_size,
        accelerator: receiver.receiver.input.accelerator,
    };
    let mut fft = FftEngine::new(settings)?;

    let base_idx = if rt.is_real {
        0usize
    } else {
        (rt.fft_size / 2) + 1
    };

    let mut wf: Option<WaterfallOffload> = if use_waterfall_thread {
        let channels = spawn_waterfall_worker(state.clone(), receiver.clone())
            .context("spawn waterfall worker")?;
        let spectrum_len = if rt.is_real {
            rt.fft_size / 2
        } else {
            rt.fft_size
        };
        let WaterfallWorkerChannels {
            free_tx,
            free_rx,
            work_tx,
        } = channels;
        for _ in 0..8usize {
            let _ = free_tx.send(vec![Complex32::new(0.0, 0.0); spectrum_len]);
        }
        Some(WaterfallOffload {
            free_tx,
            free_rx,
            work_tx,
        })
    } else {
        None
    };

    let mut frame_num: u64 = 0;
    let skip_num = {
        let frame_rate = (rt.sps as f64) / ((rt.fft_size / 2) as f64);
        // At very high sample rates, skip waterfall more aggressively to reduce load.
        // Target ~10 waterfall updates per second maximum.
        let target_wf_rate = 10.0;
        let skip = (frame_rate / target_wf_rate).ceil() as u64;
        skip.max(1)
    };
    tracing::info!(
        skip_num,
        frame_rate = ?((rt.sps as f64) / ((rt.fft_size / 2) as f64)),
        "waterfall frame skip"
    );

    let half_len_f32 = if rt.is_real {
        rt.fft_size / 2
    } else {
        rt.fft_size
    };

    enum ReaderMode {
        Threaded {
            free_tx: std::sync::mpsc::SyncSender<Vec<f32>>,
            filled_rx: std::sync::mpsc::Receiver<Vec<f32>>,
        },
        Inline {
            reader: SampleReader<Box<dyn io::Read + Send>>,
        },
    }

    let mut reader_mode = if use_reader_thread {
        let (free_tx, free_rx) =
            std::sync::mpsc::sync_channel::<Vec<f32>>(SAMPLE_BUFFER_POOL_DEPTH);
        let (filled_tx, filled_rx) =
            std::sync::mpsc::sync_channel::<Vec<f32>>(SAMPLE_BUFFER_POOL_DEPTH);
        for _ in 0..SAMPLE_BUFFER_POOL_DEPTH {
            let _ = free_tx.send(vec![0.0f32; half_len_f32]);
        }

        let reader_name = format!("novasdr-reader-{}", receiver.receiver.id);
        let receiver_id = receiver.receiver.id.clone();
        let free_tx_for_drop = free_tx.clone();
        thread::Builder::new().name(reader_name).spawn(move || {
            let mut dropped = 0u64;
            while let Ok(mut buf) = free_rx.recv() {
                if reader.read_f32(&mut buf).is_err() {
                    break;
                }
                match filled_tx.try_send(buf) {
                    Ok(()) => {}
                    Err(std::sync::mpsc::TrySendError::Full(buf)) => {
                        dropped = dropped.saturating_add(1);
                        if dropped == 1 || dropped.is_power_of_two() {
                            tracing::warn!(
                                receiver_id = %receiver_id,
                                dropped_buffers = dropped,
                                "input reader is dropping buffers due to DSP backpressure"
                            );
                        }
                        let _ = free_tx_for_drop.send(buf);
                    }
                    Err(std::sync::mpsc::TrySendError::Disconnected(_buf)) => break,
                }
            }
        })?;

        ReaderMode::Threaded { free_tx, filled_rx }
    } else {
        tracing::info!(
            receiver_id = %receiver.receiver.id,
            "running without dedicated reader thread"
        );
        ReaderMode::Inline { reader }
    };

    let (mut half_a, mut half_b) = match &mut reader_mode {
        ReaderMode::Threaded { filled_rx, .. } => {
            let half_a = filled_rx
                .recv()
                .map_err(|_| anyhow::anyhow!("reader closed"))?;
            let half_b = filled_rx
                .recv()
                .map_err(|_| anyhow::anyhow!("reader closed"))?;
            (half_a, half_b)
        }
        ReaderMode::Inline { reader } => {
            let mut half_a = vec![0.0f32; half_len_f32];
            let mut half_b = vec![0.0f32; half_len_f32];
            reader
                .read_f32(&mut half_a)
                .context("read samples (half_a)")?;
            reader
                .read_f32(&mut half_b)
                .context("read samples (half_b)")?;
            (half_a, half_b)
        }
    };

    // For IQ input we convert interleaved f32 IQ into Complex32. Avoid per-frame allocations by
    // reusing conversion buffers.
    let mut half_a_c: Vec<Complex32> = Vec::new();
    let mut half_b_c: Vec<Complex32> = Vec::new();
    if !rt.is_real {
        let complex_len = rt.fft_size / 2;
        half_a_c.resize(complex_len, Complex32::new(0.0, 0.0));
        half_b_c.resize(complex_len, Complex32::new(0.0, 0.0));
    }

    let mut audio_bins_buf: Vec<Complex32> = Vec::new();
    loop {
        let waterfall_clients = receiver
            .waterfall_clients
            .iter()
            .map(|m| m.len())
            .sum::<usize>();
        let total_clients = receiver.audio_clients.len() + waterfall_clients;

        if rt.is_real {
            fft.load_real_half_a(&half_a);
            fft.load_real_half_b(&half_b);
        } else {
            f32_iq_to_complex_into(&half_a, &mut half_a_c);
            f32_iq_to_complex_into(&half_b, &mut half_b_c);
            fft.load_complex_half_a(&half_a_c);
            fft.load_complex_half_b(&half_b_c);
        }

        if total_clients > 0 {
            let want_waterfall = waterfall_clients > 0 && frame_num.is_multiple_of(skip_num);
            let include_waterfall_in_fft = want_waterfall && wf.is_none();
            let res = fft.execute(include_waterfall_in_fft)?;

            let spectrum = fft.spectrum_for_audio();
            send_audio(
                AudioSendContext {
                    state: &state,
                    rt: &rt,
                    receiver: &receiver,
                    base_idx,
                },
                spectrum,
                frame_num,
                &mut audio_bins_buf,
            );

            if let Some(wf_offload) = wf.as_mut() {
                if want_waterfall {
                    if include_waterfall_in_fft {
                        if let (Some(quantized_concat), Some(offsets)) = (
                            res.quantized_concat.as_ref(),
                            res.quantized_level_offsets.as_ref(),
                        ) {
                            let job = WaterfallJob::Send {
                                frame_num,
                                quantized_concat: quantized_concat.clone(),
                                offsets: offsets.clone(),
                            };
                            match wf_offload.work_tx.try_send(job) {
                                Ok(()) => {}
                                Err(std::sync::mpsc::TrySendError::Full(_job)) => {}
                                Err(std::sync::mpsc::TrySendError::Disconnected(_job)) => {}
                            }
                        }
                    } else if let Ok(mut buf) = wf_offload.free_rx.try_recv() {
                        if buf.len() == spectrum.len() {
                            buf.copy_from_slice(spectrum);
                            let job = WaterfallJob::QuantizeAndSend {
                                frame_num,
                                spectrum: buf,
                                normalize: res.normalize,
                                base_idx,
                                downsample_levels: rt.downsample_levels,
                                size_log2: (rt.fft_size.ilog2() as i32) + rt.brightness_offset,
                                is_real: rt.is_real,
                            };
                            match wf_offload.work_tx.try_send(job) {
                                Ok(()) => {}
                                Err(std::sync::mpsc::TrySendError::Full(job)) => {
                                    if let WaterfallJob::QuantizeAndSend { spectrum, .. } = job {
                                        let _ = wf_offload.free_tx.send(spectrum);
                                    }
                                }
                                Err(std::sync::mpsc::TrySendError::Disconnected(job)) => {
                                    if let WaterfallJob::QuantizeAndSend { spectrum, .. } = job {
                                        let _ = wf_offload.free_tx.send(spectrum);
                                    }
                                }
                            }
                        } else {
                            let _ = wf_offload.free_tx.send(buf);
                        }
                    }
                }
            } else if want_waterfall {
                if let (Some(quantized_concat), Some(offsets)) = (
                    res.quantized_concat.as_ref(),
                    res.quantized_level_offsets.as_ref(),
                ) {
                    send_waterfall(&state, &rt, &receiver, quantized_concat, offsets, frame_num);
                }
            }
            frame_num = frame_num.wrapping_add(1);
        }

        // Shift buffers and get next one (reader is already reading ahead)
        match &mut reader_mode {
            ReaderMode::Threaded { free_tx, filled_rx } => {
                let old_a = half_a;
                half_a = half_b;
                half_b = filled_rx
                    .recv()
                    .map_err(|_| anyhow::anyhow!("reader closed"))?;
                let _ = free_tx.send(old_a);
            }
            ReaderMode::Inline { reader } => {
                std::mem::swap(&mut half_a, &mut half_b);
                reader
                    .read_f32(&mut half_b)
                    .context("read samples (half_b)")?;
            }
        }
    }
}

enum WaterfallJob {
    QuantizeAndSend {
        frame_num: u64,
        spectrum: Vec<Complex32>,
        normalize: f32,
        base_idx: usize,
        downsample_levels: usize,
        size_log2: i32,
        is_real: bool,
    },
    Send {
        frame_num: u64,
        quantized_concat: Arc<[i8]>,
        offsets: Arc<[usize]>,
    },
}

struct WaterfallOffload {
    free_tx: std::sync::mpsc::SyncSender<Vec<Complex32>>,
    free_rx: std::sync::mpsc::Receiver<Vec<Complex32>>,
    work_tx: std::sync::mpsc::SyncSender<WaterfallJob>,
}

struct WaterfallWorkerChannels {
    free_tx: std::sync::mpsc::SyncSender<Vec<Complex32>>,
    free_rx: std::sync::mpsc::Receiver<Vec<Complex32>>,
    work_tx: std::sync::mpsc::SyncSender<WaterfallJob>,
}

fn spawn_waterfall_worker(
    state: Arc<AppState>,
    receiver: Arc<ReceiverState>,
) -> anyhow::Result<WaterfallWorkerChannels> {
    let (free_tx, free_rx) = std::sync::mpsc::sync_channel::<Vec<Complex32>>(8);
    let (work_tx, work_rx) = std::sync::mpsc::sync_channel::<WaterfallJob>(4);
    let receiver_id = receiver.receiver.id.clone();
    let free_tx_for_worker = free_tx.clone();

    thread::Builder::new()
        .name(format!("novasdr-wf-{receiver_id}"))
        .spawn(move || {
            #[cfg(feature = "vkfft")]
            let mut vkfft_quantizer: Option<VkfftWaterfallQuantizer> =
                if matches!(
                    receiver.receiver.input.accelerator,
                    novasdr_core::config::Accelerator::Vkfft
                ) {
                    let fft_size = receiver.rt.fft_result_size;
                    match VkfftWaterfallQuantizer::new(fft_size) {
                        Ok(q) => Some(q),
                        Err(e) => {
                            tracing::warn!(
                                receiver_id = %receiver_id,
                                error = %e,
                                "vkfft waterfall quantizer init failed; falling back to CPU"
                            );
                            None
                        }
                    }
                } else {
                    None
                };
            #[cfg(feature = "vkfft")]
            let mut warned_gpu_quantize_failed = false;

            while let Ok(job) = work_rx.recv() {
                match job {
                    WaterfallJob::QuantizeAndSend {
                        frame_num,
                        spectrum,
                        normalize,
                        base_idx,
                        downsample_levels,
                        size_log2,
                        is_real,
                    } => {
                        let fft_result_size = spectrum.len();
                        if fft_result_size == 0 {
                            let _ = free_tx_for_worker.send(spectrum);
                            continue;
                        }

                        let base_idx = if is_real { 0 } else { base_idx % fft_result_size };
                        let (q, o) = {
                            #[cfg(feature = "vkfft")]
                            {
                                if let Some(q) = vkfft_quantizer.as_mut() {
                                    match q.quantize_and_downsample(
                                        &spectrum,
                                        base_idx,
                                        downsample_levels,
                                        size_log2,
                                        normalize,
                                    ) {
                                        Ok(res) => res,
                                        Err(e) => {
                                            if !warned_gpu_quantize_failed {
                                                warned_gpu_quantize_failed = true;
                                                tracing::warn!(
                                                    receiver_id = %receiver_id,
                                                    error = %e,
                                                    "vkfft waterfall quantize failed; falling back to CPU"
                                                );
                                            }
                                            novasdr_core::dsp::fft::quantize_and_downsample_cpu(
                                                &spectrum,
                                                normalize,
                                                base_idx,
                                                downsample_levels,
                                                size_log2,
                                            )
                                        }
                                    }
                                } else {
                                    novasdr_core::dsp::fft::quantize_and_downsample_cpu(
                                        &spectrum,
                                        normalize,
                                        base_idx,
                                        downsample_levels,
                                        size_log2,
                                    )
                                }
                            }
                            #[cfg(not(feature = "vkfft"))]
                            {
                                novasdr_core::dsp::fft::quantize_and_downsample_cpu(
                                    &spectrum,
                                    normalize,
                                    base_idx,
                                    downsample_levels,
                                    size_log2,
                                )
                            }
                        };
                        let quantized_concat: Arc<[i8]> = q.into();
                        let offsets: Arc<[usize]> = o.into();
                        let rt = receiver.rt.clone();
                        send_waterfall(
                            &state,
                            &rt,
                            &receiver,
                            &quantized_concat,
                            &offsets,
                            frame_num,
                        );

                        let _ = free_tx_for_worker.send(spectrum);
                    }
                    WaterfallJob::Send {
                        frame_num,
                        quantized_concat,
                        offsets,
                    } => {
                        let rt = receiver.rt.clone();
                        send_waterfall(
                            &state,
                            &rt,
                            &receiver,
                            &quantized_concat,
                            &offsets,
                            frame_num,
                        );
                    }
                }
            }
        })?;

    Ok(WaterfallWorkerChannels {
        free_tx,
        free_rx,
        work_tx,
    })
}

fn f32_iq_to_complex_into(interleaved: &[f32], out: &mut [Complex32]) {
    debug_assert_eq!(interleaved.len(), out.len() * 2);
    let mut i = 0usize;
    for dst in out.iter_mut() {
        let re = interleaved[i];
        let im = interleaved[i + 1];
        *dst = Complex32::new(re, im);
        i += 2;
    }
}

struct AudioSendContext<'a> {
    state: &'a Arc<AppState>,
    rt: &'a novasdr_core::config::Runtime,
    receiver: &'a Arc<ReceiverState>,
    base_idx: usize,
}

fn send_audio(
    ctx: AudioSendContext<'_>,
    spectrum: &[Complex32],
    frame_num: u64,
    bins_buf: &mut Vec<Complex32>,
) {
    let fft_result_size = ctx.rt.fft_result_size;
    for entry in ctx.receiver.audio_clients.iter() {
        let params = match entry.params.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => {
                tracing::error!(
                    unique_id = %entry.unique_id,
                    "audio params mutex poisoned; recovering"
                );
                poisoned.into_inner().clone()
            }
        };
        let l = params.l.max(0) as usize;
        let r = params.r.max(0) as usize;
        if r <= l || r > fft_result_size {
            continue;
        }
        let len = r - l;
        if len > ctx.rt.audio_max_fft_size {
            continue;
        }
        let idx = (l + ctx.base_idx) % fft_result_size;

        // Pass raw unnormalized FFT bins to the audio demod path.
        bins_buf.resize(len, Complex32::new(0.0, 0.0));
        for k in 0..len {
            bins_buf[k] = spectrum[(idx + k) % fft_result_size];
        }
        let slice = bins_buf.as_slice();
        let audio_mid_idx = params.m.floor() as i32;

        let mut pipeline = match entry.pipeline.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::error!(
                    unique_id = %entry.unique_id,
                    "audio pipeline mutex poisoned; recovering"
                );
                poisoned.into_inner()
            }
        };
        match pipeline.process(slice, frame_num, &params, ctx.rt.is_real, audio_mid_idx) {
            Ok(pkts) => {
                for pkt in pkts {
                    ctx.state
                        .total_audio_bits
                        .fetch_add(pkt.len() * 8, Ordering::Relaxed);
                    match entry.tx.try_send(pkt) {
                        Ok(()) => {}
                        Err(TokioTrySendError::Closed(_)) => {}
                        Err(TokioTrySendError::Full(_)) => {
                            ctx.state
                                .dropped_audio_frames
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "audio pipeline error");
            }
        }
    }
}

fn send_waterfall(
    state: &Arc<AppState>,
    rt: &novasdr_core::config::Runtime,
    receiver: &Arc<ReceiverState>,
    quantized_concat: &Arc<[i8]>,
    offsets: &Arc<[usize]>,
    frame_num: u64,
) {
    for (level, offset) in offsets.iter().copied().enumerate() {
        let level_len = rt.fft_result_size >> level;
        if offset + level_len > quantized_concat.len() {
            continue;
        }
        let clients = &receiver.waterfall_clients[level];
        for entry in clients.iter() {
            let p = match entry.params.lock() {
                Ok(g) => g.clone(),
                Err(poisoned) => {
                    tracing::error!("waterfall params mutex poisoned; recovering");
                    poisoned.into_inner().clone()
                }
            };
            if p.r <= p.l || p.r > level_len {
                continue;
            }
            let start = offset + p.l;
            let end = offset + p.r;
            if end > quantized_concat.len() || start >= end {
                continue;
            }

            let work = crate::state::WaterfallWorkItem {
                frame_num,
                level: p.level,
                l: p.l,
                r: p.r,
                quantized_concat: quantized_concat.clone(),
                quantized_offset: start,
            };

            match entry.tx.try_send(work) {
                Ok(()) => {}
                Err(TokioTrySendError::Closed(_)) => {}
                Err(TokioTrySendError::Full(_)) => {
                    state
                        .dropped_waterfall_frames
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

fn start_events_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick += 1;

            let wf_bits = state.total_waterfall_bits.swap(0, Ordering::Relaxed) as u64;
            let au_bits = state.total_audio_bits.swap(0, Ordering::Relaxed) as u64;
            state
                .waterfall_kbits_per_sec
                .store(wf_bits / 1000, Ordering::Relaxed);
            state
                .audio_kbits_per_sec
                .store(au_bits / 1000, Ordering::Relaxed);

            let include_changes = state.cfg.server.otherusers > 0
                && state
                    .receivers
                    .values()
                    .any(|rx| !rx.signal_changes.is_empty());
            if !include_changes && !tick.is_multiple_of(10) {
                continue;
            }
            let info = state.event_info(include_changes);
            let json = match serde_json::to_string(&info) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = ?e, "failed to serialize events payload");
                    "{}".to_string()
                }
            };
            let msg: Arc<str> = Arc::from(json);
            let mut dead = Vec::new();
            for entry in state.event_clients.iter() {
                if entry.value().try_send(msg.clone()).is_err() {
                    dead.push(*entry.key());
                }
            }
            for id in dead {
                state.event_clients.remove(&id);
            }
        }
    });
}
