// Derived from SongRec (https://github.com/marin-m/SongRec) by marin-m,
// licensed GPL-3.0-or-later.

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::sync::Arc;

use super::hanning::HANNING_WINDOW_2048_MULTIPLIERS;
use super::signature_format::{DecodedSignature, FrequencyBand, FrequencyPeak};
use crate::error::CoreError;

pub struct SignatureGenerator {
    ring_buffer_of_samples: Box<[i16; 2048]>,
    ring_buffer_of_samples_index: usize,
    reordered_ring_buffer_of_samples: Box<[f32; 2048]>,
    complex_fft_output: Box<[Complex<f32>; 1025]>,
    fft_outputs: Box<[[f32; 1025]; 256]>,
    fft_outputs_index: u8,
    fft_forward: Arc<dyn realfft::RealToComplex<f32>>,
    spread_fft_outputs: Box<[[f32; 1025]; 256]>,
    spread_fft_outputs_index: u8,
    num_spread_ffts_done: u32,
    signature: DecodedSignature,
}

impl Default for SignatureGenerator {
    fn default() -> Self {
        Self::new()
    }
}

fn alloc_boxed_array<T: Clone, const N: usize>(val: T) -> Box<[T; N]> {
    vec![val; N]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("Failed to allocate heap array of size {}", N))
}

impl SignatureGenerator {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(2048);
        Self {
            ring_buffer_of_samples: alloc_boxed_array(0i16),
            ring_buffer_of_samples_index: 0,
            reordered_ring_buffer_of_samples: alloc_boxed_array(0.0f32),
            complex_fft_output: alloc_boxed_array(Complex::new(0.0, 0.0)),
            fft_outputs: alloc_boxed_array([0.0f32; 1025]),
            fft_outputs_index: 0,
            fft_forward,
            spread_fft_outputs: alloc_boxed_array([0.0f32; 1025]),
            spread_fft_outputs_index: 0,
            num_spread_ffts_done: 0,
            signature: DecodedSignature::new(16000, 0),
        }
    }

    /// Resets the internal state to process a fresh audio segment.
    fn reset(&mut self, num_samples: u32) {
        self.ring_buffer_of_samples.fill(0);
        self.ring_buffer_of_samples_index = 0;
        self.reordered_ring_buffer_of_samples.fill(0.0);
        self.complex_fft_output.fill(Complex::new(0.0, 0.0));
        self.fft_outputs.fill([0.0; 1025]);
        self.fft_outputs_index = 0;
        self.spread_fft_outputs.fill([0.0; 1025]);
        self.spread_fft_outputs_index = 0;
        self.num_spread_ffts_done = 0;
        self.signature = DecodedSignature::new(16000, num_samples);
    }

    /// Generates a Shazam signature Data URI from contiguous 16kHz mono PCM i16 samples, reusing internal buffers.
    pub fn generate_signature(&mut self, buffer: &[i16]) -> Result<String, CoreError> {
        if buffer.len() < 128 * 46 {
            return Err(CoreError::BufferTooShort {
                expected: 128 * 46,
                actual: buffer.len(),
            });
        }

        self.reset(buffer.len() as u32);

        let chunks_count = buffer.len() / 128;
        for i in 0..chunks_count {
            let chunk = &buffer[i * 128..(i + 1) * 128];
            let mut array_chunk = [0i16; 128];
            array_chunk.copy_from_slice(chunk);
            self.do_fft(&array_chunk);
            self.do_peak_spreading();
            self.num_spread_ffts_done += 1;
            if self.num_spread_ffts_done >= 46 {
                self.do_peak_recognition();
            }
        }

        self.signature.encode_to_uri()
    }

    /// Generates a Shazam signature Data URI, compatible with the legacy SignatureGenerator interface.
    pub fn generate_from_i16(&self, buffer: &[i16]) -> Option<String> {
        Self::make_signature(buffer).ok()
    }

    /// Convenience one-shot function to generate a Shazam signature Data URI from PCM samples.
    pub fn make_signature(buffer: &[i16]) -> Result<String, CoreError> {
        let mut generator = Self::new();
        generator.generate_signature(buffer)
    }

    fn do_fft(&mut self, s16_mono_16khz_buffer: &[i16; 128]) {
        self.ring_buffer_of_samples
            [self.ring_buffer_of_samples_index..self.ring_buffer_of_samples_index + 128]
            .copy_from_slice(s16_mono_16khz_buffer);

        self.ring_buffer_of_samples_index += 128;
        self.ring_buffer_of_samples_index &= 2047;

        for (index, multiplier) in HANNING_WINDOW_2048_MULTIPLIERS.iter().enumerate() {
            self.reordered_ring_buffer_of_samples[index] = self.ring_buffer_of_samples
                [(index + self.ring_buffer_of_samples_index) & 2047]
                as f32
                * multiplier;
        }

        self.fft_forward
            .process(
                &mut *self.reordered_ring_buffer_of_samples,
                &mut *self.complex_fft_output,
            )
            .unwrap();

        let real_fft_results = &mut self.fft_outputs[self.fft_outputs_index as usize];
        for (result, complex) in real_fft_results
            .iter_mut()
            .zip(self.complex_fft_output.iter())
        {
            *result =
                ((complex.re.powi(2) + complex.im.powi(2)) / ((1 << 17) as f32)).max(0.0000000001);
        }

        self.fft_outputs_index = self.fft_outputs_index.wrapping_add(1);
    }

    fn do_peak_spreading(&mut self) {
        let real_fft_results = &self.fft_outputs[self.fft_outputs_index.wrapping_sub(1) as usize];
        let spread_fft_results =
            &mut self.spread_fft_outputs[self.spread_fft_outputs_index as usize];

        spread_fft_results.copy_from_slice(real_fft_results);

        for position in 0..=1022 {
            spread_fft_results[position] = spread_fft_results[position]
                .max(spread_fft_results[position + 1])
                .max(spread_fft_results[position + 2]);
        }

        let spread_fft_results_copy = *spread_fft_results;
        for position in 0..=1024 {
            for former_fft_number in [1, 3, 6] {
                let former_fft_output = &mut self.spread_fft_outputs[self
                    .spread_fft_outputs_index
                    .wrapping_sub(former_fft_number)
                    as usize];

                former_fft_output[position] =
                    former_fft_output[position].max(spread_fft_results_copy[position]);
            }
        }

        self.spread_fft_outputs_index = self.spread_fft_outputs_index.wrapping_add(1);
    }

    fn do_peak_recognition(&mut self) {
        let fft_minus_46 = &self.fft_outputs[self.fft_outputs_index.wrapping_sub(46) as usize];
        let fft_minus_49 =
            &self.spread_fft_outputs[self.spread_fft_outputs_index.wrapping_sub(49) as usize];

        for bin_position in 10..=1014 {
            if fft_minus_46[bin_position] >= 1.0 / 64.0
                && fft_minus_46[bin_position] >= fft_minus_49[bin_position - 1]
            {
                let mut max_neighbor_in_fft_minus_49: f32 = 0.0;
                for neighbor_offset in &[-10, -7, -4, -3, 1, 2, 5, 8] {
                    max_neighbor_in_fft_minus_49 = max_neighbor_in_fft_minus_49
                        .max(fft_minus_49[(bin_position as i32 + *neighbor_offset) as usize]);
                }

                if fft_minus_46[bin_position] > max_neighbor_in_fft_minus_49 {
                    let mut max_neighbor_in_other_adjacent_ffts = max_neighbor_in_fft_minus_49;
                    for other_offset in [
                        -53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
                    ] {
                        let other_fft = &self.spread_fft_outputs[((self.spread_fft_outputs_index
                            as i32
                            + other_offset)
                            & 255)
                            as usize];

                        max_neighbor_in_other_adjacent_ffts =
                            max_neighbor_in_other_adjacent_ffts.max(other_fft[bin_position - 1]);
                    }

                    if fft_minus_46[bin_position] > max_neighbor_in_other_adjacent_ffts {
                        let fft_pass_number = self.num_spread_ffts_done - 46;

                        let peak_magnitude: f32 =
                            fft_minus_46[bin_position].ln().max(1.0 / 64.0) * 1477.3 + 6144.0;
                        let peak_magnitude_before: f32 =
                            fft_minus_46[bin_position - 1].ln().max(1.0 / 64.0) * 1477.3 + 6144.0;
                        let peak_magnitude_after: f32 =
                            fft_minus_46[bin_position + 1].ln().max(1.0 / 64.0) * 1477.3 + 6144.0;

                        let peak_variation_1: f32 =
                            peak_magnitude * 2.0 - peak_magnitude_before - peak_magnitude_after;
                        if peak_variation_1 <= 0.0 {
                            continue;
                        }
                        let peak_variation_2: f32 = (peak_magnitude_after - peak_magnitude_before)
                            * 32.0
                            / peak_variation_1;

                        let corrected_peak_frequency_bin: u16 =
                            ((bin_position as i32 * 64) + (peak_variation_2 as i32)) as u16;

                        let frequency_hz: f32 =
                            corrected_peak_frequency_bin as f32 * (16000.0 / 2.0 / 1024.0 / 64.0);

                        let frequency_band = match frequency_hz as i32 {
                            250..=519 => FrequencyBand::_250_520,
                            520..=1449 => FrequencyBand::_520_1450,
                            1450..=3499 => FrequencyBand::_1450_3500,
                            3500..=5500 => FrequencyBand::_3500_5500,
                            _ => {
                                continue;
                            }
                        };

                        self.signature.frequency_band_to_sound_peaks[frequency_band as usize].push(
                            FrequencyPeak {
                                fft_pass_number,
                                peak_magnitude: peak_magnitude as u16,
                                corrected_peak_frequency_bin,
                            },
                        );
                    }
                }
            }
        }
    }
}
