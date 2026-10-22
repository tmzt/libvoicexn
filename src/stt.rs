//! Speech-to-text (docs/design/20261022_02_STT_MOSHI_XN_PLAN.md) on the
//! 24 kHz mono ring that [`crate::capture::Mic`] fills.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::config::{SttConfig, SttQuant};
use crate::playback::Resampler;

/// Mimi's input rate.
pub const STT_RATE: u32 = 24_000;

/// How long the microphone stays muted after voicexn stops speaking: the
/// output's own buffer and the room's echo.
pub const ECHO_HOLD_S: f32 = 0.3;

/// What the microphone has seen, for the log; read and reset by [`take`](Self::take).
#[derive(Default)]
pub struct CaptureStats {
    /// The largest absolute sample since the last `take`, as `f32` bits.
    pub(crate) peak: AtomicU32,
    /// Samples dropped because the ring was full, since the stream opened.
    pub(crate) dropped: AtomicU64,
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

/// One ASR step, whatever the LM's quantization.
trait AsrStep: Send {
    fn step(
        &mut self,
        pcm: &xn::streaming::StreamTensor<f32, xn::CpuDevice>,
        mask: &xn::streaming::StreamMask,
    ) -> xn::Result<Vec<xn_moshi::asr::StepResult>>;

    /// Attaches a token booster (plan 20261022_04, decision 5).
    #[cfg(feature = "stt-moshi-xn-boost")]
    fn set_booster(&mut self, booster: Box<dyn xn_moshi::asr::TokenBooster>) -> xn::Result<()>;
}

impl<Q: xn::BackendQ<T = f32, B = xn::CpuDevice>> AsrStep for xn_moshi::asr::AsrState<Q>
where
    xn_moshi::asr::AsrState<Q>: Send,
{
    fn step(
        &mut self,
        pcm: &xn::streaming::StreamTensor<f32, xn::CpuDevice>,
        mask: &xn::streaming::StreamMask,
    ) -> xn::Result<Vec<xn_moshi::asr::StepResult>> {
        self.step_pcm(pcm, mask, |_, _, _| {})
    }

    #[cfg(feature = "stt-moshi-xn-boost")]
    fn set_booster(&mut self, booster: Box<dyn xn_moshi::asr::TokenBooster>) -> xn::Result<()> {
        self.reset_batch_idx(0, None, None, Some(booster), None)
    }
}

/// Kyutai STT on the CPU: the LM from a quantized GGUF, Mimi from
/// `mimi.safetensors` (the GGUF leaves out Mimi's decoder, which
/// `Mimi::load` builds too).
pub(crate) struct Stt {
    state: Box<dyn AsrStep>,
    mask: xn::streaming::StreamMask,
    decoder: crate::spm::Decoder,
    /// Every word's tokens so far, with the separator before each, so that
    /// SentencePiece places the spaces (as xn-moshi's `run_asr` does).
    tokens: Vec<u32>,
    decoded: usize,
    pub(crate) asr_delay_in_tokens: usize,
}

/// Mimi's frame: 80 ms at 24 kHz.
pub(crate) const FRAME: usize = 1920;
/// Put back between words before decoding.
const WORD_SEPARATOR: u32 = 3;

/// One step's output.
#[derive(Debug, Default)]
pub(crate) struct Step {
    /// Text added by the words recognised in this step.
    pub(crate) text: String,
    pub(crate) word_starts: Vec<f64>,
}

impl Stt {
    pub(crate) fn load(settings: &SttConfig) -> Result<Self, String> {
        use xn::nn::VB;
        use xn_moshi::{lm::LmModel, mimi::Mimi, moshi};

        let dir = &settings.model_dir;

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
        let file = settings.quant.file();
        let gguf = std::fs::File::open(dir.join(file)).map_err(|e| fail(file, &e))?;
        let lm_vb =
            VB::load_gguf(std::io::BufReader::new(gguf), xn::CPU).map_err(|e| fail(file, &e))?;
        let lm_vb = lm_vb.root().pp("lm");
        let lm_config = config.to_lm_config();
        fn state<Q: xn::BackendQ<T = f32, B = xn::CpuDevice>>(
            lm_vb: &xn::nn::Path<xn::CpuDevice>,
            lm_config: &xn_moshi::lm::Config,
            delay: usize,
            mimi: Mimi<f32, xn::CpuDevice>,
        ) -> xn::Result<Box<dyn AsrStep>>
        where
            xn_moshi::asr::AsrState<Q>: Send,
        {
            let lm: LmModel<Q> = LmModel::load(lm_vb, lm_config)?;
            let asr = xn_moshi::asr::Asr::new(delay, 0.0, mimi, lm);
            Ok(Box::new(asr.init_state(1)?))
        }
        let delay = asr_delay_in_tokens;
        let state = match settings.quant {
            SttQuant::Q8_0 => state::<xn::quantized::Q80F32>(&lm_vb, &lm_config, delay, mimi),
            SttQuant::Q4_0 => state::<xn::quantized::Q40F32>(&lm_vb, &lm_config, delay, mimi),
            SttQuant::Q4k => state::<xn::quantized::Q4kF32>(&lm_vb, &lm_config, delay, mimi),
        }
        .map_err(|e| fail("lm", &e))?;
        log::info!("stt: LM {file}");
        #[cfg(feature = "stt-moshi-xn-boost")]
        let mut state = state;
        #[cfg(feature = "stt-moshi-xn-boost")]
        if let Some(path) = &settings.vocab {
            let phrases = crate::boost::read_vocab(path).map_err(|e| fail("vocab", &e))?;
            let (start, cont) = settings.boost;
            let booster = crate::boost::VocabBooster::from_phrases(&decoder, &phrases, start, cont);
            log::info!(
                "stt: boosting {} phrases ({} token sequences) from {}, start {start}, cont {cont}",
                phrases.len(),
                booster.len(),
                path.display()
            );
            state
                .set_booster(Box::new(booster))
                .map_err(|e| fail("booster", &e))?;
        }
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
    pub(crate) fn step(&mut self, frame: &[f32; FRAME]) -> Result<Step, String> {
        let audio = xn::Tensor::from_vec(frame.to_vec(), (1, 1, FRAME), &xn::CPU)
            .map_err(|e| e.to_string())?;
        let pcm = xn::streaming::StreamTensor::from_tensor(audio);
        let results = self
            .state
            .step(&pcm, &self.mask)
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
    pub(crate) fn text(&self) -> String {
        self.decoder.decode(&self.tokens)
    }
}

/// Loads the model and transcribes a WAV file the way xn-moshi's `run_asr`
/// does: two frames of silence before it, and the ASR delay's worth after.
/// Logs each word as it comes and the step times against the 80 ms budget.
pub fn transcribe_file(config: &SttConfig, wav: &std::path::Path) -> Result<String, String> {
    use std::time::Instant;

    let pcm = read_wav(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let start = Instant::now();
    let mut stt = Stt::load(config)?;
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
        "stt: {} load {load_ms} ms, {} steps, step mean {mean:.1} ms, p95 {:.1} ms, max {:.1} ms (budget 80 ms), {} threads",
        config.quant.file(),
        steps.len(),
        sorted[sorted.len() * 95 / 100],
        sorted[sorted.len() - 1],
        std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "default".into()),
    );
    Ok(stt.text())
}

/// A 16-bit PCM or 32-bit float WAV file as 24 kHz mono.
pub(crate) fn read_wav(path: &std::path::Path) -> Result<Vec<f32>, String> {
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

/// Splits the transcript into utterances: one ends once a word has been
/// heard and then [`Segmenter::end_frames`] frames pass without another.
pub(crate) struct Segmenter {
    pub(crate) end_frames: u32,
    text: String,
    quiet: u32,
}

impl Segmenter {
    /// About 1.2 s of no new words, after the model's own 0.5 s delay.
    pub(crate) fn new() -> Self {
        Self {
            end_frames: 15,
            text: String::new(),
            quiet: 0,
        }
    }

    /// Ends the utterance now, if it has any words.
    #[cfg(feature = "vad-earshot")]
    pub(crate) fn flush(&mut self) -> Option<String> {
        self.quiet = 0;
        let text = std::mem::take(&mut self.text);
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// The utterance so far.
    pub(crate) fn partial(&self) -> &str {
        self.text.trim()
    }

    /// Takes one step's new text; returns the utterance once it ends.
    pub(crate) fn push(&mut self, new_text: &str) -> Option<String> {
        if !new_text.is_empty() {
            self.text.push_str(new_text);
            self.quiet = 0;
            return None;
        }
        if self.text.trim().is_empty() {
            return None;
        }
        self.quiet += 1;
        if self.quiet < self.end_frames {
            return None;
        }
        self.quiet = 0;
        Some(std::mem::take(&mut self.text).trim().to_string())
    }
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum SttEvent {
    Loaded {
        load_ms: u32,
    },
    LoadFailed(String),
    /// The utterance so far, after a step that added words.
    Partial(String),
    /// An utterance that has ended.
    Utterance(String),
    /// Every 25 frames (2 s of audio): the mean step time against the
    /// 80 ms budget, and the samples the capture has dropped.
    Speed {
        step_ms: f32,
        dropped: u64,
    },
    /// The tap-to-talk gate changed state (plan 20261022_03).
    #[cfg(feature = "vad-earshot")]
    Gate(crate::vad::GateState),
}

/// What the app asks of the `stt` thread (plan 20261022_03, decision 5).
#[cfg(feature = "vad-earshot")]
#[derive(Clone, Copy, Debug)]
pub enum SttRequest {
    Arm,
    Close,
}

/// The `stt` thread: it loads the model, then steps it on each 80 ms frame
/// the capture ring holds (plan 20261022_02, decision 4), or with
/// `vad-earshot`, on what the gate lets through.
pub struct SttWorker {
    rx: std::sync::mpsc::Receiver<SttEvent>,
    #[cfg(feature = "vad-earshot")]
    tx: std::sync::mpsc::Sender<SttRequest>,
}

impl SttWorker {
    pub fn spawn(config: SttConfig, capture: Capture) -> Self {
        let (events, rx) = std::sync::mpsc::channel();
        #[cfg(feature = "vad-earshot")]
        let (tx, requests) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("stt".into())
            .spawn(move || {
                let Some(mut listener) = Listener::load(&config, capture, events) else {
                    return;
                };
                #[cfg(feature = "vad-earshot")]
                listener.run_gated(&requests);
                #[cfg(not(feature = "vad-earshot"))]
                listener.run();
            })
            .expect("spawning the stt thread");
        Self {
            rx,
            #[cfg(feature = "vad-earshot")]
            tx,
        }
    }

    pub fn poll(&self) -> impl Iterator<Item = SttEvent> + '_ {
        self.rx.try_iter()
    }

    #[cfg(feature = "vad-earshot")]
    pub fn send(&self, req: SttRequest) {
        let _ = self.tx.send(req);
    }
}

/// The `stt` thread's state once the model is loaded.
struct Listener {
    stt: Stt,
    capture: Capture,
    events: std::sync::mpsc::Sender<SttEvent>,
    segmenter: Segmenter,
    /// Audio waiting to make up an 80 ms frame.
    pending: Vec<f32>,
    steps: u32,
    step_time: std::time::Duration,
    dropped_at_load: u64,
}

/// The app has gone: the thread ends.
struct Gone;

impl Listener {
    fn load(
        config: &SttConfig,
        mut capture: Capture,
        events: std::sync::mpsc::Sender<SttEvent>,
    ) -> Option<Self> {
        let start = std::time::Instant::now();
        let stt = match Stt::load(config) {
            Ok(stt) => stt,
            Err(e) => {
                let _ = events.send(SttEvent::LoadFailed(e));
                return None;
            }
        };
        let _ = events.send(SttEvent::Loaded {
            load_ms: start.elapsed().as_millis() as u32,
        });
        // What was said while the model loaded is not transcribed, and what
        // the full ring dropped meanwhile is not counted.
        let n = capture.ring.slots();
        if let Ok(chunk) = capture.ring.read_chunk(n) {
            chunk.commit_all();
        }
        let dropped_at_load = capture.stats.take().1;
        Some(Self {
            stt,
            capture,
            events,
            segmenter: Segmenter::new(),
            pending: Vec::with_capacity(2 * FRAME),
            steps: 0,
            step_time: std::time::Duration::ZERO,
            dropped_at_load,
        })
    }

    fn send(&self, event: SttEvent) -> Result<(), Gone> {
        self.events.send(event).map_err(|_| Gone)
    }

    /// Reads up to `n` samples from the capture ring into `out`, or waits a
    /// little and returns `false` if fewer than `n` are there.
    fn read(&mut self, n: usize, out: &mut Vec<f32>) -> bool {
        let Ok(chunk) = self.capture.ring.read_chunk(n) else {
            std::thread::sleep(std::time::Duration::from_millis(5));
            return false;
        };
        let (a, b) = chunk.as_slices();
        out.clear();
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        chunk.commit_all();
        true
    }

    /// Feeds audio to STT, stepping each full 80 ms frame. Returns the
    /// utterance if one ended.
    fn feed(&mut self, audio: &[f32]) -> Result<Option<String>, Gone> {
        use std::time::Instant;

        self.pending.extend_from_slice(audio);
        let mut ended = None;
        while self.pending.len() >= FRAME {
            let mut frame = [0.0; FRAME];
            frame.copy_from_slice(&self.pending[..FRAME]);
            self.pending.drain(..FRAME);
            let t = Instant::now();
            let step = match self.stt.step(&frame) {
                Ok(step) => step,
                Err(e) => {
                    log::error!("stt: {e}");
                    continue;
                }
            };
            self.step_time += t.elapsed();
            self.steps += 1;
            let added = !step.text.is_empty();
            if let Some(text) = self.segmenter.push(&step.text) {
                self.send(SttEvent::Utterance(text.clone()))?;
                ended = Some(text);
            } else if added {
                self.send(SttEvent::Partial(self.segmenter.partial().to_string()))?;
            }
            if self.steps == 25 {
                let dropped = self.capture.stats.take().1 - self.dropped_at_load;
                self.send(SttEvent::Speed {
                    step_ms: self.step_time.as_secs_f32() * 1000.0 / self.steps as f32,
                    dropped,
                })?;
                (self.steps, self.step_time) = (0, std::time::Duration::ZERO);
            }
        }
        Ok(ended)
    }

    /// Always listening: every frame goes to STT.
    #[cfg(not(feature = "vad-earshot"))]
    fn run(&mut self) {
        let mut frame = Vec::with_capacity(FRAME);
        loop {
            if self.read(FRAME, &mut frame) && self.feed(&frame).is_err() {
                return;
            }
        }
    }

    /// Tap to talk (plan 20261022_03): earshot scores 16 ms frames while the
    /// gate listens, and STT hears only what the gate lets through.
    #[cfg(feature = "vad-earshot")]
    fn run_gated(&mut self, requests: &std::sync::mpsc::Receiver<SttRequest>) {
        use crate::vad::{Gate, GateState, SUB};

        let mut gate = Gate::new();
        let mut detector = earshot::Detector::default();
        let mut to_16k = Resampler::new(STT_RATE, 16_000);
        let (mut sub, mut feed, mut at_16k) = (Vec::new(), Vec::new(), Vec::new());
        let mut score = 0.0;
        let result: Result<(), Gone> = (|| loop {
            for req in requests.try_iter() {
                let changed = match req {
                    SttRequest::Arm => {
                        detector.reset();
                        to_16k.reset();
                        at_16k.clear();
                        score = 0.0;
                        gate.arm()
                    }
                    SttRequest::Close => gate.close(),
                };
                if let Some(state) = changed {
                    self.send(SttEvent::Gate(state))?;
                }
            }
            if !self.read(SUB, &mut sub) || !gate.listening() {
                continue; // idle: the frame is dropped unheard
            }
            to_16k.process(&sub, &mut at_16k);
            while at_16k.len() >= 256 {
                score = detector.predict_f32(&at_16k[..256]);
                at_16k.drain(..256);
            }
            feed.clear();
            let mut changed = gate.push(score, &sub, &mut feed);
            if self.feed(&feed)?.is_some() {
                changed = gate.finish().or(changed);
            }
            if changed == Some(GateState::Idle) {
                // Closed without an utterance: what was heard still counts.
                if let Some(text) = self.segmenter.flush() {
                    self.send(SttEvent::Utterance(text))?;
                }
                self.pending.clear();
            }
            if let Some(state) = changed {
                self.send(SttEvent::Gate(state))?;
            }
        })();
        let _ = result;
    }
}

#[cfg(test)]
mod segmenter_tests {
    use super::*;

    #[test]
    fn an_utterance_ends_after_quiet_frames() {
        let mut s = Segmenter::new();
        assert_eq!(s.push(""), None, "no words yet: nothing to end");
        for _ in 0..40 {
            assert_eq!(s.push(""), None);
        }
        assert_eq!(s.push(" Hello"), None);
        assert_eq!(s.push(" there."), None);
        assert_eq!(s.partial(), "Hello there.");
        for _ in 0..14 {
            assert_eq!(s.push(""), None);
        }
        assert_eq!(s.push("").as_deref(), Some("Hello there."));
        assert_eq!(s.partial(), "");
        assert_eq!(s.push(""), None);
    }

    #[test]
    fn a_word_restarts_the_quiet_count() {
        let mut s = Segmenter::new();
        s.push(" one");
        for _ in 0..10 {
            s.push("");
        }
        s.push(" two");
        for _ in 0..14 {
            assert_eq!(s.push(""), None);
        }
        assert_eq!(s.push("").as_deref(), Some("one two"));
    }
}
