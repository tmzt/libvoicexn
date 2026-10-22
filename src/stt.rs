//! Speech-to-text (docs/design/20261022_02_STT_MOSHI_XN_PLAN.md). The
//! capture sink turns the microphone into 24 kHz mono for Mimi and pushes
//! it into a ring, without blocking the input thread (decision 5); while
//! voicexn speaks it pushes silence instead (decision 6).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use libmorph::{CaptureFormat, Captured, Sink};

use crate::playback::Resampler;

/// Mimi's input rate.
pub const STT_RATE: u32 = 24_000;

/// What the sink has seen, for the log; read and reset by [`take`](Self::take).
#[derive(Default)]
pub struct CaptureStats {
    /// The largest absolute sample since the last `take`, as `f32` bits.
    peak: AtomicU32,
    /// Samples dropped because the ring was full, since the stream opened.
    dropped: AtomicU64,
}

impl CaptureStats {
    /// The peak since the last call, and the total dropped.
    pub fn take(&self) -> (f32, u64) {
        let peak = f32::from_bits(self.peak.swap(0, Ordering::Relaxed));
        (peak, self.dropped.load(Ordering::Relaxed))
    }
}

/// The reading end of the capture: 24 kHz mono.
pub struct Capture {
    pub ring: rtrb::Consumer<f32>,
    pub stats: Arc<CaptureStats>,
}

/// The sink for `Audio::add_sink`, and the ring it fills (`seconds` long).
/// While `speaking` is set, the sink pushes zeros in place of what it hears.
pub fn capture(format: CaptureFormat, speaking: Arc<AtomicBool>, seconds: u32) -> (Sink, Capture) {
    let (mut ring, consumer) = rtrb::RingBuffer::new((STT_RATE * seconds) as usize);
    let stats = Arc::new(CaptureStats::default());
    let seen = stats.clone();
    let channels = usize::from(format.channels.max(1));
    let mut resampler = Resampler::new(format.sample_rate, STT_RATE);
    // Room for blocks of up to a second; larger ones grow it once.
    let mut mono = Vec::with_capacity(format.sample_rate as usize);
    let mut out = Vec::with_capacity(STT_RATE as usize);
    let sink = Box::new(move |captured: &Captured<'_>| {
        mono.clear();
        let mut peak = 0.0f32;
        for frame in captured.samples.chunks_exact(channels) {
            let x = frame.iter().sum::<f32>() / channels as f32;
            peak = peak.max(x.abs());
            mono.push(x);
        }
        seen.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        if speaking.load(Ordering::Relaxed) {
            mono.fill(0.0);
        }
        out.clear();
        resampler.process(&mono, &mut out);
        let n = out.len().min(ring.slots());
        if let Ok(mut chunk) = ring.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            for (slot, &x) in a.iter_mut().chain(b.iter_mut()).zip(&out) {
                slot.write(x);
            }
            // SAFETY: all `n` slots were written above.
            unsafe { chunk.commit_all() };
        }
        if n < out.len() {
            seen.dropped
                .fetch_add((out.len() - n) as u64, Ordering::Relaxed);
        }
    });
    (
        sink,
        Capture {
            ring: consumer,
            stats,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn block(samples: &[f32]) -> Captured<'_> {
        Captured {
            samples,
            frame: 0,
            time: Duration::ZERO,
        }
    }

    #[test]
    fn stereo_48k_arrives_as_mono_24k_and_silence_while_speaking() {
        let format = CaptureFormat {
            sample_rate: 48_000,
            channels: 2,
        };
        let speaking = Arc::new(AtomicBool::new(false));
        let (mut sink, mut capture) = capture(format, speaking.clone(), 1);
        // Left 0.5, right -0.1: mono 0.2.
        let stereo: Vec<f32> = [0.5, -0.1].repeat(480);
        sink(&block(&stereo));
        let n = capture.ring.slots();
        assert!((239..=240).contains(&n), "{n}");
        let chunk = capture.ring.read_chunk(n).unwrap();
        let (a, _) = chunk.as_slices();
        assert!(
            a[1..].iter().all(|&x| (x - 0.2).abs() < 1e-6),
            "{:?}",
            &a[..4]
        );
        chunk.commit_all();
        assert_eq!(capture.stats.take(), (0.2, 0));

        speaking.store(true, Ordering::Relaxed);
        sink(&block(&stereo));
        let n = capture.ring.slots();
        let chunk = capture.ring.read_chunk(n).unwrap();
        assert!(chunk.as_slices().0.iter().all(|&x| x == 0.0));
        chunk.commit_all();
        // The level is still measured: it is the microphone, not the ring.
        assert_eq!(capture.stats.take().0, 0.2);
    }

    #[test]
    fn a_full_ring_drops_and_counts() {
        let format = CaptureFormat {
            sample_rate: 24_000,
            channels: 1,
        };
        let (mut sink, capture) = capture(format, Arc::new(AtomicBool::new(false)), 1);
        let second = vec![0.0; 24_000];
        sink(&block(&second));
        sink(&block(&second));
        assert!(capture.stats.take().1 >= 23_999);
    }
}

/// Kyutai STT on the CPU, q8: the LM from `model.q8.gguf`, Mimi from
/// `mimi.safetensors` (the GGUF leaves out Mimi's decoder, which
/// `Mimi::load` builds too).
pub struct Stt {
    state: xn_moshi::asr::AsrState<xn::quantized::Q80F32>,
    mask: xn::streaming::StreamMask,
    decoder: crate::spm::Decoder,
    /// Every word's tokens so far, with the separator before each, so that
    /// SentencePiece places the spaces (as xn-moshi's `run_asr` does).
    tokens: Vec<u32>,
    decoded: usize,
    pub asr_delay_in_tokens: usize,
}

/// Mimi's frame: 80 ms at 24 kHz.
pub const FRAME: usize = 1920;
/// Put back between words before decoding.
const WORD_SEPARATOR: u32 = 3;

/// One step's output.
#[derive(Debug, Default)]
pub struct Step {
    /// Text added by the words recognised in this step.
    pub text: String,
    pub word_starts: Vec<f64>,
}

impl Stt {
    pub fn load(dir: &std::path::Path) -> Result<Self, String> {
        use xn::nn::VB;
        use xn_moshi::{lm::LmModel, mimi::Mimi, moshi};

        let fail =
            |what: &str, e: &dyn std::fmt::Display| format!("{}: {what}: {e}", dir.display());
        let config = std::fs::read_to_string(dir.join("config.json"))
            .map_err(|e| fail("config.json", &e))?;
        let raw: serde_json::Value =
            serde_json::from_str(&config).map_err(|e| fail("config.json", &e))?;
        let config: moshi::Config =
            serde_json::from_value(raw.clone()).map_err(|e| fail("config.json", &e))?;
        let decoder =
            crate::spm::Decoder::open(&dir.join("tokenizer.model")).map_err(|e| e.to_string())?;

        let mimi_config = xn_moshi::mimi::Config::v0_1(Some(32));
        // The model hears each word this long before it writes it: 0.5 s
        // for stt-1b-en_fr, in 80 ms frames.
        let delay_s = raw["stt_config"]["audio_delay_seconds"]
            .as_f64()
            .unwrap_or(0.5);
        let asr_delay_in_tokens = (delay_s * 12.5).round() as usize;
        let mimi_vb =
            VB::load(&[dir.join("mimi.safetensors")], xn::CPU).map_err(|e| fail("mimi", &e))?;
        let mimi: Mimi<f32, xn::CpuDevice> =
            Mimi::load(&mimi_vb.root(), mimi_config).map_err(|e| fail("mimi", &e))?;
        let gguf = std::fs::File::open(dir.join("model.q8.gguf"))
            .map_err(|e| fail("model.q8.gguf", &e))?;
        let lm_vb = VB::load_gguf(std::io::BufReader::new(gguf), xn::CPU)
            .map_err(|e| fail("model.q8.gguf", &e))?;
        let lm: LmModel<xn::quantized::Q80F32> =
            LmModel::load(&lm_vb.root().pp("lm"), &config.to_lm_config())
                .map_err(|e| fail("lm", &e))?;
        let asr = xn_moshi::asr::Asr::new(asr_delay_in_tokens, 0.0, mimi, lm);
        let state = asr.init_state(1).map_err(|e| fail("state", &e))?;
        Ok(Self {
            state,
            mask: xn::streaming::StreamMask::all_active(1),
            decoder,
            tokens: Vec::new(),
            decoded: 0,
            asr_delay_in_tokens,
        })
    }

    /// Feeds one 80 ms frame of 24 kHz mono.
    pub fn step(&mut self, frame: &[f32; FRAME]) -> Result<Step, String> {
        let audio = xn::Tensor::from_vec(frame.to_vec(), (1, 1, FRAME), &xn::CPU)
            .map_err(|e| e.to_string())?;
        let pcm = xn::streaming::StreamTensor::from_tensor(audio);
        let results = self
            .state
            .step_pcm(&pcm, &self.mask, |_, _, _| {})
            .map_err(|e| e.to_string())?;
        let mut step = Step::default();
        for result in results {
            for word in result.words {
                if let xn_moshi::asr::AsrWord::Word {
                    tokens, start_time, ..
                } = word
                {
                    self.tokens.push(WORD_SEPARATOR);
                    self.tokens.extend_from_slice(&tokens);
                    step.word_starts.push(start_time);
                }
            }
        }
        if !step.word_starts.is_empty() {
            let text = self.decoder.decode(&self.tokens);
            step.text = text.get(self.decoded..).unwrap_or_default().to_string();
            self.decoded = text.len();
        }
        Ok(step)
    }

    /// Everything recognised so far.
    pub fn text(&self) -> String {
        self.decoder.decode(&self.tokens)
    }
}

/// Loads the model and transcribes a WAV file the way xn-moshi's `run_asr`
/// does: two frames of silence before it, and the ASR delay's worth after.
/// Logs each word as it comes and the step times against the 80 ms budget.
pub fn transcribe_file(model: &std::path::Path, wav: &std::path::Path) -> Result<String, String> {
    use std::time::Instant;

    let pcm = read_wav(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let start = Instant::now();
    let mut stt = Stt::load(model)?;
    let load_ms = start.elapsed().as_millis();
    let audio = [
        vec![0.0; FRAME * 2],
        pcm,
        vec![0.0; FRAME * stt.asr_delay_in_tokens],
    ]
    .concat();
    let mut steps = Vec::new();
    for chunk in audio.chunks(FRAME) {
        let mut frame = [0.0; FRAME];
        frame[..chunk.len()].copy_from_slice(chunk);
        let t = Instant::now();
        let step = stt.step(&frame)?;
        steps.push(t.elapsed().as_secs_f64() * 1000.0);
        if !step.text.is_empty() {
            log::info!("stt: +{:?}", step.text);
        }
    }
    let mean = steps.iter().sum::<f64>() / steps.len() as f64;
    let mut sorted = steps.clone();
    sorted.sort_by(f64::total_cmp);
    log::info!(
        "stt: load {load_ms} ms, {} steps, step mean {mean:.1} ms, p95 {:.1} ms, max {:.1} ms (budget 80 ms), {} threads",
        steps.len(),
        sorted[sorted.len() * 95 / 100],
        sorted[sorted.len() - 1],
        std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "default".into()),
    );
    Ok(stt.text())
}

/// A 16-bit PCM or 32-bit float WAV file as 24 kHz mono.
pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut format, mut channels, mut rate, mut bits, mut data) = (0, 0, 0, 0, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let data = data.ok_or("no data chunk")?;
    let samples: Vec<f32> = match (format, bits) {
        (1, 16) => data
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        _ => return Err(format!("unsupported WAV format {format}, {bits} bits")),
    };
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    let mut out = Vec::new();
    Resampler::new(rate, STT_RATE).process(&mono, &mut out);
    Ok(out)
}
