use crate::audio::resampler::AudioResampler;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SAMPLE_RATE: usize = 16000;
const BUFFER_SECS: usize = 12;
const RING_CAPACITY: usize = SAMPLE_RATE * BUFFER_SECS; // 192,000 samples

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSourceMode {
    Auto,
    Monitor,
    Mic,
}

pub struct RingBuffer {
    data: Vec<i16>,
    write_idx: usize,
    count: usize,
}

impl RingBuffer {
    pub fn new() -> Self {
        Self {
            data: vec![0i16; RING_CAPACITY],
            write_idx: 0,
            count: 0,
        }
    }

    pub fn push_slice(&mut self, samples: &[i16]) {
        for &s in samples {
            self.data[self.write_idx] = s;
            self.write_idx = (self.write_idx + 1) % RING_CAPACITY;
            if self.count < RING_CAPACITY {
                self.count += 1;
            }
        }
    }

    pub fn extract_recent(&self, num_samples: usize) -> Vec<i16> {
        let n = num_samples.min(self.count);
        let mut out = Vec::with_capacity(n);

        if self.count < RING_CAPACITY {
            let start = self.write_idx.saturating_sub(n);
            out.extend_from_slice(&self.data[start..self.write_idx]);
        } else {
            let start_idx = (self.write_idx + RING_CAPACITY - n) % RING_CAPACITY;
            if start_idx + n <= RING_CAPACITY {
                out.extend_from_slice(&self.data[start_idx..start_idx + n]);
            } else {
                let first_part = RING_CAPACITY - start_idx;
                let second_part = n - first_part;
                out.extend_from_slice(&self.data[start_idx..RING_CAPACITY]);
                out.extend_from_slice(&self.data[0..second_part]);
            }
        }

        out
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn clear(&mut self) {
        self.write_idx = 0;
        self.count = 0;
    }
}

pub struct AudioCapture {
    is_running: Arc<AtomicBool>,
    ring_buffer: Arc<Mutex<RingBuffer>>,
    shutdown: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn new(mode: AudioSourceMode) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let is_running = Arc::new(AtomicBool::new(true));
        let ring_buffer = Arc::new(Mutex::new(RingBuffer::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let is_running_clone = is_running.clone();
        let ring_buffer_clone = ring_buffer.clone();
        let shutdown_clone = shutdown.clone();

        std::thread::Builder::new()
            .name("hark-audio-capture".into())
            .spawn(move || {
                Self::capture_loop(mode, is_running_clone, ring_buffer_clone, shutdown_clone);
            })?;

        Ok(Self {
            is_running,
            ring_buffer,
            shutdown,
        })
    }

    fn select_device(host: &cpal::Host, mode: AudioSourceMode) -> Option<cpal::Device> {
        match mode {
            AudioSourceMode::Mic | AudioSourceMode::Auto => host.default_input_device(),
            AudioSourceMode::Monitor => {
                // Look for an input device containing "monitor"
                if let Ok(devices) = host.input_devices() {
                    for dev in devices {
                        if let Ok(name) = dev.name() {
                            if name.to_lowercase().contains("monitor") {
                                return Some(dev);
                            }
                        }
                    }
                }
                // Fallback to default input
                host.default_input_device()
            }
        }
    }

    fn capture_loop(
        mode: AudioSourceMode,
        is_running: Arc<AtomicBool>,
        ring_buffer: Arc<Mutex<RingBuffer>>,
        shutdown: Arc<AtomicBool>,
    ) {
        while !shutdown.load(Ordering::Relaxed) {
            // Wait until we are supposed to be running before opening any stream.
            // This ensures the hardware mic is fully released when paused.
            if !is_running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }

            let host = cpal::default_host();
            let device = match Self::select_device(&host, mode) {
                Some(d) => d,
                None => {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let channels = config.channels();
            let sample_rate = config.sample_rate().0;

            let device_changed = Arc::new(AtomicBool::new(false));
            let device_changed_clone = device_changed.clone();

            let err_fn = move |err| {
                eprintln!("Audio stream notification: {}", err);
                device_changed_clone.store(true, Ordering::Relaxed);
            };

            let ring_clone = ring_buffer.clone();

            let stream_result = match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    let ring = ring_clone.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[f32], _| {
                            let pcm16 =
                                AudioResampler::resample_to_16k_mono(data, channels, sample_rate);
                            if let Ok(mut lock) = ring.lock() {
                                lock.push_slice(&pcm16);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::I16 => {
                    let ring = ring_clone.clone();
                    device.build_input_stream(
                        &config.into(),
                        move |data: &[i16], _| {
                            let f32_data: Vec<f32> =
                                data.iter().map(|&s| s as f32 / 32768.0).collect();
                            let pcm16 = AudioResampler::resample_to_16k_mono(
                                &f32_data,
                                channels,
                                sample_rate,
                            );
                            if let Ok(mut lock) = ring.lock() {
                                lock.push_slice(&pcm16);
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                _ => {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to build input stream: {}. Retrying...", e);
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            if let Err(e) = stream.play() {
                eprintln!("Failed to start audio stream: {}. Retrying...", e);
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            // Stream actively captures until shutdown, device change, or pause
            while !shutdown.load(Ordering::Relaxed)
                && !device_changed.load(Ordering::Relaxed)
                && is_running.load(Ordering::Relaxed)
            {
                std::thread::sleep(Duration::from_millis(100));
            }

            // Drop the stream to fully release the hardware mic
            drop(stream);

            if device_changed.load(Ordering::Relaxed) {
                // Allow audio server / WirePlumber to settle after device unplug/plug
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }

    pub fn pause(&self) {
        self.is_running.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.is_running.store(true, Ordering::Relaxed);
    }

    pub fn extract_chunk(&self, duration_secs: usize) -> Vec<i16> {
        let lock = self.ring_buffer.lock().unwrap_or_else(|e| e.into_inner());
        let samples_wanted = duration_secs * SAMPLE_RATE;
        lock.extract_recent(samples_wanted)
    }

    pub fn sample_count(&self) -> usize {
        let lock = self.ring_buffer.lock().unwrap_or_else(|e| e.into_inner());
        lock.count()
    }

    pub fn clear_buffer(&self) {
        let mut lock = self.ring_buffer.lock().unwrap_or_else(|e| e.into_inner());
        lock.clear();
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}
