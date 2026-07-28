use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use hound::{WavSpec, WavWriter};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "linux")]
use pulsectl::controllers::DeviceControl;

#[cfg(target_os = "windows")]
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::PROPERTYKEY;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::{
    eCapture, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    STGM_READ,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

#[derive(serde::Serialize, Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub label: String,
}

#[cfg(target_os = "windows")]
unsafe fn get_string_property(props: &IPropertyStore, key: *const PROPERTYKEY) -> Option<String> {
    let mut pv = match props.GetValue(key) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let result = match PropVariantToStringAlloc(&pv) {
        Ok(pwstr) => {
            if pwstr.is_null() {
                None
            } else {
                let s = pwstr.to_string().ok();
                let _ = CoTaskMemFree(Some(pwstr.0 as *const _));
                s
            }
        }
        Err(_) => None,
    };

    let _ = PropVariantClear(&mut pv);
    result.filter(|s| !s.trim().is_empty())
}

#[cfg(target_os = "windows")]
fn get_windows_audio_devices() -> Result<Vec<AudioDevice>, String> {
    let mut devices = Vec::new();
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() && hr.0 != 0x00040101 {
            // RPC_E_CHANGED_MODE
            crate::log_info!("Windows Audio: CoInitializeEx failed: {:?}", hr);
        }

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("Failed to create MMDeviceEnumerator: {}", e))?;

        let collection = enumerator
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("Failed to enum audio endpoints: {}", e))?;

        let count = collection
            .GetCount()
            .map_err(|e| format!("Failed to get device count: {}", e))?;

        crate::log_info!("Windows Audio: Found {} active capture endpoints", count);

        for i in 0..count {
            if let Ok(device) = collection.Item(i) {
                let id_pwstr = match device.GetId() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let id = id_pwstr.to_string().unwrap_or_default();
                CoTaskMemFree(Some(id_pwstr.0 as *const _));

                if let Ok(props) = device.OpenPropertyStore(STGM_READ) {
                    let friendly_name = get_string_property(&props, &PKEY_Device_FriendlyName);
                    let device_desc = get_string_property(&props, &PKEY_Device_DeviceDesc);

                    if friendly_name.is_none() && device_desc.is_none() {
                        if let Ok(p_count) = props.GetCount() {
                            crate::log_info!("Windows Audio: Store for {} has {} properties but FriendlyName/Desc missing", id, p_count);
                            for j in 0..p_count {
                                let mut pk = PROPERTYKEY::default();

                                if props.GetAt(j, &mut pk).is_ok() {
                                    crate::log_info!(
                                        "   Property {}: GUID={:?}, PID={}",
                                        j,
                                        pk.fmtid,
                                        pk.pid
                                    );
                                }
                            }
                        }
                    }

                    let friendly = friendly_name.unwrap_or_else(|| "Unknown Device".to_string());
                    let label = if let Some(desc) = device_desc {
                        let f_lower = friendly.to_lowercase();
                        let d_lower = desc.to_lowercase();

                        if f_lower.contains(&d_lower) {
                            friendly
                        } else if d_lower.contains(&f_lower) {
                            desc
                        } else {
                            format!("{} - {}", friendly, desc)
                        }
                    } else if friendly == "Unknown Device" {
                        format!("Unknown Device ({})", id)
                    } else {
                        friendly
                    };

                    crate::log_info!("Windows Audio: Enumerated '{}'", label);
                    devices.push(AudioDevice { id, label });
                }
            }
        }
    }
    Ok(devices)
}

pub struct PersistentAudioEngine {
    pub stream: cpal::Stream,
    /// A true sliding window of the last ~200ms of audio - a plain `VecDeque` under a
    /// mutex rather than `ringbuf`, since `ringbuf`'s overwrite-on-full push
    /// (`push_overwrite`) is only available on an unsplit buffer, not the
    /// producer/consumer-split handles this needs (one written from the realtime audio
    /// callback, one read from wherever a recording/utterance starts).
    pub pre_roll_buffer: Arc<Mutex<std::collections::VecDeque<f32>>>,
    pub recording_tx: Arc<Mutex<Option<mpsc::SyncSender<f32>>>>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PersistentAudioEngine {
    pub fn new(device: &cpal::Device, sensitivity: f32) -> Result<Self, String> {
        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;
        let sample_rate = u32::from(config.sample_rate());
        let channels = config.channels();

        crate::log_info!(
            "Audio Engine: Opening native stream ({}Hz, {} channels)",
            sample_rate,
            channels
        );

        let pre_roll_size = (sample_rate as f32 * 0.2) as usize;
        let pre_roll_buffer = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::with_capacity(
            pre_roll_size,
        )));
        let pre_roll_buffer_clone = pre_roll_buffer.clone();

        let recording_tx = Arc::new(Mutex::new(None::<mpsc::SyncSender<f32>>));
        let recording_tx_clone = recording_tx.clone();

        let err_fn = |err| crate::log_info!("Audio stream error: {}", err);

        let stream_config: cpal::StreamConfig = config.clone().into();
        let channels_usize = channels as usize;

        let audio_callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for frame in data.chunks(channels_usize) {
                // Sum all channels to mono.
                // Note: We don't divide by channel count here to preserve volume from single-channel mics
                // reporting as stereo. Clipping is handled by the soft-clipper later for Whisper,
                // but for raw testing we want the full energy.
                let sample_raw: f32 = frame.iter().sum();

                // Apply manual sensitivity/volume
                let sample = sample_raw * sensitivity;

                // True sliding window of "the last ~200ms": push the newest, drop the
                // oldest once over capacity, so it never goes stale. `try_lock` (not
                // `lock`) like the recording_tx send below - this runs on the realtime
                // audio callback thread, which must never block on a mutex.
                if let Ok(mut buf) = pre_roll_buffer_clone.try_lock() {
                    buf.push_back(sample);
                    if buf.len() > pre_roll_size {
                        buf.pop_front();
                    }
                }
                if let Ok(guard) = recording_tx_clone.try_lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.try_send(sample);
                    }
                }
            }
        };

        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                device.build_input_stream(&stream_config, audio_callback, err_fn, None)
            }
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], info| {
                    let f32_data: Vec<f32> =
                        data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    audio_callback(&f32_data, info);
                },
                err_fn,
                None,
            ),
            _ => return Err("Unsupported sample format".into()),
        }
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        Ok(Self {
            stream,
            pre_roll_buffer,
            recording_tx,
            sample_rate,
            channels,
        })
    }
}

#[cfg(target_os = "linux")]
fn get_linux_pulse_devices() -> Result<Vec<AudioDevice>, String> {
    let mut devices = Vec::new();
    let mut handler = pulsectl::controllers::SourceController::create()
        .map_err(|e| format!("Failed to connect to PulseAudio: {}", e))?;
    let sources = handler
        .list_devices()
        .map_err(|e| format!("Failed to list PulseAudio sources: {}", e))?;

    for source in sources {
        let name = source.name.clone().unwrap_or_default();
        let description = source.description.clone().unwrap_or_default();
        if name.to_lowercase().contains(".monitor")
            || description.to_lowercase().contains("monitor")
        {
            continue;
        }
        devices.push(AudioDevice {
            id: format!("pulse:{}", name),
            label: description,
        });
    }
    Ok(devices)
}

pub fn get_input_devices() -> Result<Vec<AudioDevice>, String> {
    let mut final_devices = Vec::new();
    #[cfg(target_os = "linux")]
    {
        if let Ok(devices) = get_linux_pulse_devices() {
            final_devices = devices;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(devices) = get_windows_audio_devices() {
            final_devices = devices;
        }
    }

    if final_devices.is_empty() {
        let mut seen_labels = std::collections::HashMap::new();
        for host_id in cpal::available_hosts() {
            if let Ok(host) = cpal::host_from_id(host_id) {
                if let Ok(devices) = host.input_devices() {
                    for dev in devices {
                        let id = match dev.id() {
                            Ok(id) => id.1,
                            Err(_) => continue,
                        };
                        #[cfg(target_os = "linux")]
                        if !id.starts_with("default:") && id != "pulse" && id != "default" {
                            continue;
                        }

                        let mut label = match dev.description() {
                            Ok(desc) => desc.name().to_string(),
                            Err(_) => id.clone(),
                        };

                        let count = seen_labels.entry(label.clone()).or_insert(0);
                        *count += 1;
                        if *count > 1 {
                            label = format!("{} ({})", label, *count);
                        }
                        final_devices.push(AudioDevice { id, label });
                    }
                }
            }
        }
    }

    final_devices.sort_by(|a, b| a.label.cmp(&b.label));
    final_devices.insert(
        0,
        AudioDevice {
            id: "default".to_string(),
            label: "System Default".to_string(),
        },
    );
    Ok(final_devices)
}

pub fn lookup_device(target_id: Option<String>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let target = target_id.filter(|id| id != "default");

    fn summarize_input_devices(host: &cpal::Host) -> String {
        match host.input_devices() {
            Ok(devices) => devices
                .map(|device| {
                    let identifier = device
                        .id()
                        .map(|id| id.1)
                        .unwrap_or_else(|_| "<unknown-id>".to_string());
                    let label = device
                        .description()
                        .map(|description| description.name().to_string())
                        .unwrap_or_else(|_| "<unknown-name>".to_string());
                    format!("{} ({})", identifier, label)
                })
                .collect::<Vec<String>>()
                .join(", "),
            Err(error) => format!("<failed to enumerate input devices: {}>", error),
        }
    }

    #[cfg(target_os = "linux")]
    fn summarize_pulse_sources() -> String {
        let mut controller = match pulsectl::controllers::SourceController::create() {
            Ok(controller) => controller,
            Err(error) => return format!("<failed to connect PulseAudio: {}>", error),
        };

        match controller.list_devices() {
            Ok(sources) => sources
                .into_iter()
                .map(|source| {
                    source
                        .name
                        .unwrap_or_else(|| "<unknown-source>".to_string())
                })
                .collect::<Vec<String>>()
                .join(", "),
            Err(error) => format!("<failed to list PulseAudio sources: {}>", error),
        }
    }

    let available_inputs = summarize_input_devices(&host);

    if let Some(name) = target {
        if let Some(_stripped) = name.strip_prefix("pulse:") {
            #[cfg(target_os = "linux")]
            {
                std::env::set_var("PULSE_SOURCE", _stripped);
            }

            host.default_input_device().ok_or_else(|| {
                #[cfg(target_os = "linux")]
                {
                    let pulse_sources = summarize_pulse_sources();
                    return format!(
                        "Failed to resolve Pulse source '{}': no default input device available after setting PULSE_SOURCE. pulse_sources=[{}], input_devices=[{}]",
                        _stripped, pulse_sources, available_inputs
                    );
                }

                #[cfg(not(target_os = "linux"))]
                {
                    format!(
                        "Failed to resolve device '{}': no default input device available. input_devices=[{}]",
                        name, available_inputs
                    )
                }
            })
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|d| d.id().map(|id| id.1 == name).unwrap_or(false))
                .ok_or_else(|| {
                    format!(
                        "Device '{}' not found. input_devices=[{}]",
                        name, available_inputs
                    )
                })
        }
    } else {
        #[cfg(target_os = "linux")]
        {
            if let Ok(devices) = host.input_devices() {
                for dev in devices {
                    if let Ok(id) = dev.id() {
                        if id.1 == "pulse" || id.1.starts_with("default") {
                            return Ok(dev);
                        }
                    }
                }
            }
        }
        host.default_input_device().ok_or_else(|| {
            format!(
                "No input device available. input_devices=[{}]",
                available_inputs
            )
        })
    }
}

pub async fn record_audio_while_flag(
    is_recording: &Arc<Mutex<bool>>,
    engine: Arc<Mutex<Option<PersistentAudioEngine>>>,
    post_roll_ms: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    crate::log_info!("record_audio_while_flag: enter");
    let (tx, rx) = mpsc::sync_channel::<f32>(65536);
    let mut samples = Vec::new();
    let sample_rate;
    {
        let mut guard = engine.lock().unwrap();
        let eng = guard.as_mut().ok_or("Audio engine not initialized")?;
        sample_rate = eng.sample_rate;
        if let Ok(mut buf) = eng.pre_roll_buffer.lock() {
            samples.extend(buf.drain(..));
        }
        *eng.recording_tx.lock().unwrap() = Some(tx);
    }

    let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut all = samples;
        while let Ok(s) = rx.recv() {
            all.push(s);
        }
        let mut out = Vec::new();
        if let Ok(mut w) = WavWriter::new(
            std::io::Cursor::new(&mut out),
            WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        ) {
            for s in all {
                let _ = w.write_sample(process_sample(s));
            }
            let _ = w.finalize();
        }
        let _ = data_tx.send(out);
    });

    while *is_recording.lock().unwrap() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    crate::log_info!("record_audio_while_flag: flag observed false, finalizing capture");

    if post_roll_ms > 0 {
        tokio::time::sleep(Duration::from_millis(post_roll_ms)).await;
        crate::log_info!("record_audio_while_flag: post-roll of {post_roll_ms}ms complete");
    }

    if let Some(eng) = engine.lock().unwrap().as_ref() {
        *eng.recording_tx.lock().unwrap() = None;
    }
    let final_wav = data_rx.recv()?;
    crate::log_info!(
        "record_audio_while_flag: captured {} bytes of wav before whisper conversion",
        final_wav.len()
    );
    convert_audio_for_whisper(&final_wav, sample_rate, 1)
}

/// Encodes raw f32 samples (already sensitivity-scaled by the input callback) into a
/// whisper-ready WAV byte buffer, exactly like `record_audio_while_flag`'s internal
/// writer, but exposed for callers - like continuous listening - that assemble their own
/// sample buffers per-utterance instead of capturing one continuous take.
fn samples_to_whisper_wav(
    samples: &[f32],
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = Vec::new();
    {
        let mut writer = WavWriter::new(
            std::io::Cursor::new(&mut out),
            WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for &s in samples {
            writer.write_sample(process_sample(s))?;
        }
        writer.finalize()?;
    }
    convert_audio_for_whisper(&out, sample_rate, 1)
}

/// Continuously listens on the given engine using simple energy-based Voice Activity
/// Detection (VAD) to auto-segment speech into utterances, instead of requiring the
/// caller to mark an explicit start/stop for each recording. Each detected utterance is
/// delivered as a complete whisper-ready WAV buffer through the returned channel as soon
/// as trailing silence confirms it's over, so the caller can transcribe one utterance
/// while this keeps listening for the next.
///
/// Stops (flushing any in-progress utterance first) once `is_recording` becomes false.
///
/// `silence_end_ms` is how long a pause has to last before an utterance is considered
/// finished (the user-facing "Pause Sensitivity" setting) - lower cuts off natural
/// mid-sentence pauses more eagerly, higher is more patient but slower to send text.
/// The speech-detection RMS threshold itself is still a fixed constant tuned by ear
/// against typical mic input after the existing sensitivity scaling.
pub fn listen_continuously(
    is_recording: Arc<Mutex<bool>>,
    engine: Arc<Mutex<Option<PersistentAudioEngine>>>,
    silence_end_ms: u64,
) -> Result<mpsc::Receiver<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let (tx, rx) = mpsc::sync_channel::<f32>(65536);
    let sample_rate;
    let pre_roll_buffer;
    {
        let mut guard = engine.lock().unwrap();
        let eng = guard.as_mut().ok_or("Audio engine not initialized")?;
        sample_rate = eng.sample_rate;
        pre_roll_buffer = eng.pre_roll_buffer.clone();
        *eng.recording_tx.lock().unwrap() = Some(tx);
    }

    let (utterance_tx, utterance_rx) = mpsc::channel::<Vec<u8>>();

    std::thread::spawn(move || {
        const SPEECH_RMS_THRESHOLD: f32 = 0.02;
        const WINDOW_MS: f32 = 30.0;
        const MIN_UTTERANCE_MS: f32 = 300.0;
        // Keep continuous dictation responsive: Whisper can process these rolling
        // windows while capture continues, letting Compose revise recent sentences
        // instead of waiting for a half-minute monologue to finish.
        const MAX_UTTERANCE_MS: f32 = 8_000.0;

        let window_size = ((sample_rate as f32) * (WINDOW_MS / 1000.0)).max(1.0) as usize;
        let silence_end_windows = ((silence_end_ms as f32) / WINDOW_MS).ceil().max(1.0) as usize;
        let min_utterance_windows = (MIN_UTTERANCE_MS / WINDOW_MS).ceil() as usize;
        let max_utterance_windows = (MAX_UTTERANCE_MS / WINDOW_MS).ceil() as usize;

        let mut window: Vec<f32> = Vec::with_capacity(window_size);
        let mut utterance: Vec<f32> = Vec::new();
        let mut in_speech = false;
        let mut trailing_silence_windows = 0usize;
        let mut utterance_windows = 0usize;

        let finalize = |utterance: &mut Vec<f32>| {
            if utterance.is_empty() {
                return;
            }
            match samples_to_whisper_wav(utterance, sample_rate) {
                Ok(wav) => {
                    let _ = utterance_tx.send(wav);
                }
                Err(error) => {
                    crate::log_info!(
                        "Continuous listen: failed to encode utterance: {}",
                        error
                    );
                }
            }
            utterance.clear();
        };

        while let Ok(sample) = rx.recv() {
            window.push(sample);
            if window.len() < window_size {
                continue;
            }

            let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
            let is_speech_window = rms > SPEECH_RMS_THRESHOLD;

            if is_speech_window {
                if !in_speech {
                    crate::log_info!("Continuous listen: speech started (rms={:.4})", rms);
                    in_speech = true;
                    utterance_windows = 0;
                    // RMS-based onset detection is inherently a little late - speech has
                    // to ramp up over a full window before it crosses the threshold - so
                    // without this, the first syllable or two gets clipped (e.g. "Let me
                    // know how it looks" transcribed as just "know how it looks"). The
                    // pre-roll buffer has been continuously filling with the last ~200ms
                    // of audio the whole time, onset or not, so prepend it now.
                    if let Ok(mut buf) = pre_roll_buffer.lock() {
                        utterance.extend(buf.drain(..));
                    }
                }
                trailing_silence_windows = 0;
            } else if in_speech {
                trailing_silence_windows += 1;
            }

            if in_speech {
                utterance.append(&mut window);
                window = Vec::with_capacity(window_size);
                utterance_windows += 1;

                let silence_long_enough = trailing_silence_windows >= silence_end_windows
                    && utterance_windows >= min_utterance_windows;
                let hit_max_length = utterance_windows >= max_utterance_windows;

                if silence_long_enough || hit_max_length {
                    if hit_max_length {
                        crate::log_info!(
                            "Continuous listen: max utterance length reached, finalizing early"
                        );
                    } else {
                        crate::log_info!("Continuous listen: silence confirmed, finalizing utterance");
                    }
                    finalize(&mut utterance);
                    in_speech = false;
                    trailing_silence_windows = 0;
                    utterance_windows = 0;
                }
            } else {
                window.clear();
            }
        }

        // Channel closed (recording stopped) - flush whatever utterance was in progress
        // rather than silently dropping the last thing the user said.
        finalize(&mut utterance);
        crate::log_info!("Continuous listen: capture thread exiting");
    });

    // Watch `is_recording` and drop the engine's sender once it flips false, which makes
    // `rx.recv()` above return Err and end the capture thread's loop - same shutdown
    // pattern as `record_audio_while_flag`.
    {
        std::thread::spawn(move || loop {
            if !*is_recording.lock().unwrap() {
                if let Some(eng) = engine.lock().unwrap().as_ref() {
                    *eng.recording_tx.lock().unwrap() = None;
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        });
    }

    Ok(utterance_rx)
}

pub async fn record_mic_test<F>(
    is_mic_test: &Arc<Mutex<bool>>,
    engine: Arc<Mutex<Option<PersistentAudioEngine>>>,
    on_volume: F,
) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>>
where
    F: Fn(f32) + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<f32>(65536);
    let sample_rate;
    {
        let mut guard = engine.lock().unwrap();
        let eng = guard.as_mut().ok_or("Audio engine not initialized")?;
        sample_rate = eng.sample_rate;
        *eng.recording_tx.lock().unwrap() = Some(tx);
    }

    let (data_tx, data_rx) = mpsc::channel::<Vec<f32>>();
    std::thread::spawn(move || {
        let mut samples = Vec::new();
        let mut peak = 0.0f32;
        let mut count = 0;
        let throttle_window = 800;
        while let Ok(s) = rx.recv() {
            let abs_s = s.abs();
            if abs_s > peak {
                peak = abs_s;
            }
            count += 1;
            if count >= throttle_window {
                on_volume(peak);
                peak = 0.0;
                count = 0;
            }
            samples.push(s);
        }
        let _ = data_tx.send(samples);
    });

    while *is_mic_test.lock().unwrap() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let Some(eng) = engine.lock().unwrap().as_ref() {
        *eng.recording_tx.lock().unwrap() = None;
    }
    let final_samples = data_rx.recv()?;

    crate::log_info!(
        "Mic test: Finished with {} samples at {}Hz",
        final_samples.len(),
        sample_rate
    );

    Ok(final_samples)
}

pub fn play_audio<F>(
    samples: Vec<f32>,
    sample_rate: u32,
    on_done: F,
) -> Result<cpal::Stream, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce() + Send + 'static,
{
    let host = cpal::default_host();
    let device = {
        let mut selected = None;
        if let Ok(devices) = host.output_devices() {
            for dev in devices {
                if let Ok(id) = dev.id() {
                    #[cfg(target_os = "linux")]
                    if id.1 == "pulse" || id.1.starts_with("default") {
                        selected = Some(dev);
                        break;
                    }
                    #[cfg(not(target_os = "linux"))]
                    if id.1.starts_with("default") {
                        selected = Some(dev);
                        break;
                    }
                }
            }
        }
        selected.or_else(|| host.default_output_device())
    }
    .ok_or("No output device available")?;

    let config = device.default_output_config()?;
    let stream_config: StreamConfig = config.clone().into();
    let resampled = Arc::new(resample_audio_f32(
        &samples,
        sample_rate,
        stream_config.sample_rate,
    ));
    let chans = stream_config.channels as usize;

    let err_fn = |err| crate::log_info!("Playback error: {}", err);
    let mut done = Some(on_done);

    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            let resampled_clone = resampled.clone();
            let mut idx = 0;
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(chans) {
                        if idx < resampled_clone.len() {
                            let s = resampled_clone[idx];
                            for out in frame.iter_mut() {
                                *out = s;
                            }
                            idx += 1;
                        } else {
                            for out in frame.iter_mut() {
                                *out = 0.0;
                            }
                            if let Some(cb) = done.take() {
                                cb();
                            }
                        }
                    }
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let resampled_clone = resampled.clone();
            let mut idx = 0;
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| {
                    for frame in data.chunks_mut(chans) {
                        if idx < resampled_clone.len() {
                            let s = (resampled_clone[idx] * i16::MAX as f32) as i16;
                            for out in frame.iter_mut() {
                                *out = s;
                            }
                            idx += 1;
                        } else {
                            for out in frame.iter_mut() {
                                *out = 0;
                            }
                            if let Some(cb) = done.take() {
                                cb();
                            }
                        }
                    }
                },
                err_fn,
                None,
            )?
        }
        _ => return Err("Unsupported format".into()),
    };
    stream.play()?;
    Ok(stream)
}

fn convert_audio_for_whisper(
    data: &[u8],
    rate: u32,
    _chans: u16,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    if rate == 16000 {
        return Ok(data.to_vec());
    }
    let mut reader = hound::WavReader::new(std::io::Cursor::new(data))?;
    let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap_or(0)).collect();
    let mut mono = if reader.spec().channels == 2 {
        samples
            .chunks(2)
            .map(|c| {
                if c.len() == 2 {
                    ((c[0] as i32 + c[1] as i32) / 2) as i16
                } else {
                    c[0]
                }
            })
            .collect()
    } else {
        samples
    };
    if reader.spec().sample_rate != 16000 {
        mono = resample_audio(&mono, reader.spec().sample_rate, 16000);
    }
    let mut out = Vec::new();
    {
        let mut w = WavWriter::new(
            std::io::Cursor::new(&mut out),
            WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for s in mono {
            w.write_sample(s)?;
        }
        w.finalize()?;
    }
    Ok(out)
}

pub fn resample_audio(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to {
        return samples.to_vec();
    }

    if from > to && from % to == 0 {
        let ratio = (from / to) as usize;
        let mut out = Vec::with_capacity(samples.len() / ratio);
        for chunk in samples.chunks_exact(ratio) {
            let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
            out.push((sum / ratio as i32) as i16);
        }
        return out;
    }

    let ratio = from as f64 / to as f64;
    let len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = pos - idx as f64;
        if idx + 1 < samples.len() {
            let s1 = samples[idx] as f64;
            let s2 = samples[idx + 1] as f64;
            out.push((s1 + (s2 - s1) * frac) as i16);
        } else if idx < samples.len() {
            out.push(samples[idx]);
        }
    }
    out
}

pub fn resample_audio_f32(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = pos - idx as f64;
        if idx + 1 < samples.len() {
            let s1 = samples[idx] as f64;
            let s2 = samples[idx + 1] as f64;
            out.push((s1 + (s2 - s1) * frac) as f32);
        } else if idx < samples.len() {
            out.push(samples[idx]);
        }
    }
    out
}

fn process_sample(s: f32) -> i16 {
    let clipped = soft_clip(s);
    (clipped * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.7 {
        x
    } else if x > 0.7 {
        0.7 + 0.3 * ((x - 0.7) / 0.3).tanh()
    } else {
        -0.7 - 0.3 * ((-x - 0.7) / 0.3).tanh()
    }
}
