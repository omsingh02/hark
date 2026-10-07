// Derived from SongRec (https://github.com/marin-m/SongRec) by marin-m,
// licensed GPL-3.0-or-later.

use base64::Engine;
use byteorder::{LittleEndian, WriteBytesExt};
use crc32fast::Hasher;
use std::io::{Cursor, Seek, SeekFrom, Write};

use crate::error::CoreError;

pub const DATA_URI_PREFIX: &str = "data:audio/vnd.shazam.sig;base64,";

#[derive(Debug, Clone)]
pub struct FrequencyPeak {
    pub fft_pass_number: u32,
    pub peak_magnitude: u16,
    pub corrected_peak_frequency_bin: u16,
}

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Debug, Clone, Copy)]
pub enum FrequencyBand {
    _250_520 = 0,
    _520_1450 = 1,
    _1450_3500 = 2,
    _3500_5500 = 3,
}

#[derive(Debug, Clone)]
pub struct DecodedSignature {
    pub sample_rate_hz: u32,
    pub number_samples: u32,
    pub frequency_band_to_sound_peaks: [Vec<FrequencyPeak>; 4],
}

impl DecodedSignature {
    pub fn new(sample_rate_hz: u32, number_samples: u32) -> Self {
        Self {
            sample_rate_hz,
            number_samples,
            frequency_band_to_sound_peaks: Default::default(),
        }
    }

    pub fn encode_to_binary(&self) -> Result<Vec<u8>, CoreError> {
        let mut cursor = Cursor::new(Vec::with_capacity(1024));

        cursor.write_u32::<LittleEndian>(0xcafe2580)?; // magic1
        cursor.write_u32::<LittleEndian>(0)?; // crc32 placeholder
        cursor.write_u32::<LittleEndian>(0)?; // size_minus_header placeholder
        cursor.write_u32::<LittleEndian>(0x94119c00)?; // magic2
        cursor.write_u32::<LittleEndian>(0)?;
        cursor.write_u32::<LittleEndian>(0)?;
        cursor.write_u32::<LittleEndian>(0)?;
        cursor.write_u32::<LittleEndian>(
            match self.sample_rate_hz {
                8000 => 1,
                11025 => 2,
                16000 => 3,
                32000 => 4,
                44100 => 5,
                48000 => 6,
                _ => 3,
            } << 27,
        )?; // shifted_sample_rate_id
        cursor.write_u32::<LittleEndian>(0)?;
        cursor.write_u32::<LittleEndian>(0)?;
        cursor.write_u32::<LittleEndian>(
            self.number_samples + (self.sample_rate_hz as f32 * 0.24) as u32,
        )?; // number_samples_plus_divided_sample_rate
        cursor.write_u32::<LittleEndian>((15 << 19) + 0x40000)?; // fixed_value: 0x7c0000

        cursor.write_u32::<LittleEndian>(0x40000000)?;
        cursor.write_u32::<LittleEndian>(0)?; // size_minus_header placeholder

        for (frequency_band, frequency_peaks) in
            self.frequency_band_to_sound_peaks.iter().enumerate()
        {
            if frequency_peaks.is_empty() {
                continue;
            }
            let mut peaks_cursor = Cursor::new(Vec::with_capacity(frequency_peaks.len() * 5));
            let mut fft_pass_number = 0;

            for frequency_peak in frequency_peaks {
                let diff = frequency_peak.fft_pass_number.saturating_sub(fft_pass_number);

                if diff >= 255 {
                    peaks_cursor.write_u8(0xff)?;
                    peaks_cursor.write_u32::<LittleEndian>(frequency_peak.fft_pass_number)?;
                    fft_pass_number = frequency_peak.fft_pass_number;
                }

                let step = (frequency_peak.fft_pass_number - fft_pass_number) as u8;
                peaks_cursor.write_u8(step)?;
                peaks_cursor.write_u16::<LittleEndian>(frequency_peak.peak_magnitude)?;
                peaks_cursor
                    .write_u16::<LittleEndian>(frequency_peak.corrected_peak_frequency_bin)?;

                fft_pass_number = frequency_peak.fft_pass_number;
            }

            let peaks_buffer = peaks_cursor.into_inner();

            cursor.write_u32::<LittleEndian>(0x60030040 + frequency_band as u32)?;
            cursor.write_u32::<LittleEndian>(peaks_buffer.len() as u32)?;
            cursor.write_all(&peaks_buffer)?;
            let padding = (4 - (peaks_buffer.len() % 4)) % 4;
            for _ in 0..padding {
                cursor.write_u8(0)?;
            }
        }

        let buffer_size = cursor.position() as u32;

        // Patch size_minus_header at offset 8 and offset 52
        cursor.seek(SeekFrom::Start(8))?;
        cursor.write_u32::<LittleEndian>(buffer_size - 48)?;

        cursor.seek(SeekFrom::Start(48 + 4))?;
        cursor.write_u32::<LittleEndian>(buffer_size - 48)?;

        // Patch CRC32 at offset 4
        cursor.seek(SeekFrom::Start(4))?;
        let mut hasher = Hasher::new();
        hasher.update(&cursor.get_ref()[8..]);
        cursor.write_u32::<LittleEndian>(hasher.finalize())?;

        Ok(cursor.into_inner())
    }

    pub fn encode_to_uri(&self) -> Result<String, CoreError> {
        let binary = self.encode_to_binary()?;
        let encoded = base64::prelude::BASE64_STANDARD.encode(binary);
        Ok(format!("{}{}", DATA_URI_PREFIX, encoded))
    }
}
