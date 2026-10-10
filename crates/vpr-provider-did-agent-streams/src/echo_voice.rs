//! Server-only synthesis via the canonical TtsPort. No TTS request in Python.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use vpr_integration::{CancellationProbe, GeneratedAudioBuffer, PcmSampleFormat, ProviderError, TtsPort, TtsRequest};
use super::{cancelled, invalid_response};

pub(super) fn encode_private_wav(tts: &dyn TtsPort, text: &str, cancellation: &dyn CancellationProbe) -> Result<String, ProviderError> {
        let mut output = GeneratedAudioBuffer::default();
        tts.synthesize(&TtsRequest { text: text.to_owned(), locale_hint: None }, cancellation, &mut output)?;
        if cancellation.is_cancelled() { return Err(cancelled()); }
        let rate = output.sample_rate_hz().ok_or_else(invalid_response)?;
        let channels = output.channels().ok_or_else(invalid_response)?;
        if output.sample_format() != Some(PcmSampleFormat::S16Le)
            || !(8_000..=96_000).contains(&rate)
            || !(1..=2).contains(&channels)
            || output.pcm().is_empty()
            || output.pcm().len() > 8 * 1024 * 1024 - 44
            || output.duration_millis().is_none_or(|ms| ms > 60_000)
        {
            return Err(invalid_response());
        }
        let byte_len = u32::try_from(output.pcm().len()).map_err(|_| invalid_response())?;
        let block_align = channels.checked_mul(2).ok_or_else(invalid_response)?;
        let bytes_per_sec = rate.checked_mul(u32::from(block_align)).ok_or_else(invalid_response)?;
        let mut wav = Vec::with_capacity(44 + output.pcm().len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + byte_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&bytes_per_sec.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&byte_len.to_le_bytes());
        wav.extend_from_slice(output.pcm());
        Ok(STANDARD.encode(wav))
    }

