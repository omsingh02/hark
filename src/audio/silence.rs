/// Silence / Energy Detector using RMS in dBFS
pub struct SilenceDetector {
    pub threshold_dbfs: f32,
}

impl Default for SilenceDetector {
    fn default() -> Self {
        Self {
            threshold_dbfs: -55.0, // Calibrated threshold for ambient microphone & room audio
        }
    }
}

impl SilenceDetector {
    #[allow(dead_code)]
    pub fn new(threshold_dbfs: f32) -> Self {
        Self { threshold_dbfs }
    }

    /// Computes RMS in dBFS for a slice of i16 PCM samples.
    /// Returns (is_silent, dbfs).
    pub fn is_silent(&self, samples: &[i16]) -> (bool, f32) {
        if samples.is_empty() {
            return (true, -100.0);
        }

        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let mean_sq = sum_sq / (samples.len() as f64);
        let rms = mean_sq.sqrt();

        // 32768.0 is max amplitude for i16
        let normalized_rms = (rms / 32768.0).max(1e-9);
        let dbfs = 20.0 * (normalized_rms as f32).log10();

        (dbfs < self.threshold_dbfs, dbfs)
    }
}
