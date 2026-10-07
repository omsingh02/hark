use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("I/O error during signature serialization: {0}")]
    Io(#[from] std::io::Error),

    #[error("Audio buffer too short: expected at least {expected} samples, got {actual}")]
    BufferTooShort { expected: usize, actual: usize },

    #[error("Invalid sample rate: {0} Hz (supported: 8000, 11025, 16000, 32000, 44100, 48000)")]
    InvalidSampleRate(u32),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
