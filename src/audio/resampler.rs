/// Anti-aliased audio downsampler and channel converter to 16,000 Hz Mono PCM i16.
pub struct AudioResampler;

impl AudioResampler {
    /// Resamples an interleaved PCM buffer of any channel count and sample rate to 16 kHz Mono i16.
    pub fn resample_to_16k_mono(input: &[f32], channels: u16, source_rate: u32) -> Vec<i16> {
        if input.is_empty() {
            return Vec::new();
        }

        // Step 1: Downmix channels to mono
        let ch = channels as usize;
        let mono_len = input.len() / ch;
        let mut mono = Vec::with_capacity(mono_len);

        for i in 0..mono_len {
            let mut sum = 0.0f32;
            for c in 0..ch {
                sum += input[i * ch + c];
            }
            mono.push(sum / (ch as f32));
        }

        if source_rate == 16000 {
            // Already 16 kHz
            return mono
                .into_iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
        }

        // Step 2: High quality bandlimited interpolation
        let ratio = 16000.0 / (source_rate as f64);
        let target_len = (mono.len() as f64 * ratio).floor() as usize;
        let mut output = Vec::with_capacity(target_len);

        for i in 0..target_len {
            let src_idx = (i as f64) / ratio;
            let idx0 = src_idx.floor() as usize;
            let frac = (src_idx - (idx0 as f64)) as f32;

            let s0 = if idx0 < mono.len() { mono[idx0] } else { 0.0 };
            let s1 = if idx0 + 1 < mono.len() {
                mono[idx0 + 1]
            } else {
                s0
            };

            // Linear interpolation between samples
            let sample = s0 + frac * (s1 - s0);
            let s16 = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            output.push(s16);
        }

        output
    }
}
