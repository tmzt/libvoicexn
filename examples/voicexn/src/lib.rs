//! voicexn, libvoicexn's demo: Phonon text-to-speech on xn, in a libmorph
//! app (docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md), and with
//! `stt-moshi-xn`, Kyutai speech-to-text on xn-moshi that hears an
//! utterance and repeats it with Phonon
//! (docs/design/20261022_02_STT_MOSHI_XN_PLAN.md). The engine is
//! libvoicexn (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md).

pub mod ui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;
#[cfg(feature = "stt-moshi-xn")]
use std::time::Instant;

use libcontrols::DrawContext;
use libmorph::{AudioSetup, Frame, LimitsRequest, MorphApp, MorphContext, Touch, TouchPhase, wgpu};
use libmsdf::GpuSdfRenderer;
use libmsdf::drawlist::DrawList;
use libmsdf::font::{
    BUNDLED_ATLAS_PX_RANGE, FontAtlas, ROBOTO_REGULAR_ASCII, ROBOTO_REGULAR_ATLAS, TextShaper,
};

use crate::ui::{Layout, Line, SPOKEN, Screen, wrap};
#[cfg(feature = "stt-moshi-xn")]
use crate::ui::{MUTED, TEXT};
#[cfg(feature = "stt-moshi-xn")]
use libvoicexn::stt;
use libvoicexn::tts::{TtsEvent, TtsRequest, TtsWorker};
#[cfg(feature = "vad-earshot")]
use libvoicexn::vad;
use libvoicexn::{morph, playback};

/// The frame rate while STT listens: enough for text, and far less heat
/// than the display's 120 Hz.
#[cfg(feature = "stt-moshi-xn")]
const LISTENING_FPS: u32 = 15;

/// Spoken on a tap until something has been heard.
const PHRASE: &str =
    "Phonon turns text into natural speech, right on your device, with no cloud round trip.";

/// Utterances kept on screen.
const HISTORY: usize = 4;

/// A checkpoint folder: `models/<name>` on the desktop, `<data dir>/<name>`
/// on Android. `--model <dir>` overrides Phonon's on the desktop.
fn model_dir(ctx: &MorphContext, name: &str) -> PathBuf {
    if let Some(dir) = ctx.data_dir() {
        return dir.join(name);
    }
    let args = ctx.args();
    let flag = args.iter().position(|a| a == "--model");
    match flag.and_then(|i| args.get(i + 1)) {
        Some(dir) if name == "phonon-31m-en" => PathBuf::from(dir),
        _ => PathBuf::from("models").join(name),
    }
}

pub struct App {
    atlas: FontAtlas,
    shaper: TextShaper,
    sdf: GpuSdfRenderer,
    list: DrawList,
    screen: Screen,
    /// When the next frame may start, while the frame rate is capped.
    #[cfg(feature = "stt-moshi-xn")]
    next_frame: Instant,
    tts: TtsWorker,
    status: String,
    /// What was heard and repeated, newest first.
    history: VecDeque<String>,
    #[cfg(feature = "stt-moshi-xn")]
    stt: Option<stt::SttWorker>,
    #[cfg(feature = "stt-moshi-xn")]
    stt_status: String,
    /// What STT is doing, for the status line: "listening", or with
    /// `vad-earshot`, the gate's state.
    #[cfg(feature = "stt-moshi-xn")]
    stt_doing: &'static str,
    #[cfg(feature = "vad-earshot")]
    gate: vad::GateState,
    /// The utterance being heard.
    #[cfg(feature = "stt-moshi-xn")]
    partial: String,
    /// Set by the audio thread while voicexn is speaking.
    #[cfg(feature = "stt-moshi-xn")]
    speaking: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl App {
    /// Gives the screen what it shows now; it changes only what differs.
    fn lay_out(&mut self, ctx: &MorphContext) {
        let (w, h) = ctx.surface_size();
        let safe = ctx.safe_area();
        let layout = Layout {
            w: w as f32,
            h: h as f32,
            scale: ctx.scale_factor(),
            top: safe.top,
            bottom: safe.bottom,
        };
        #[cfg_attr(not(feature = "stt-moshi-xn"), allow(unused_mut))]
        let mut status = vec![self.status.clone()];
        let mut body = Vec::new();
        let line = |text: String, size: f32, color, gap| Line {
            text,
            size,
            color,
            gap,
        };
        #[cfg(feature = "stt-moshi-xn")]
        {
            status.push(self.stt_status.clone());
            let size = 22.0;
            if self.partial.is_empty() {
                body.push(line("\u{2026}".into(), size, MUTED, 24.0));
            }
            for (i, text) in wrap(&self.partial, layout.columns(size))
                .into_iter()
                .enumerate()
            {
                body.push(line(text, size, TEXT, if i == 0 { 24.0 } else { 0.0 }));
            }
        }
        let size = 19.0;
        for said in &self.history {
            let lines = wrap(&format!("> {said}"), layout.columns(size));
            for (i, text) in lines.into_iter().enumerate() {
                body.push(line(text, size, SPOKEN, if i == 0 { 18.0 } else { 0.0 }));
            }
        }
        self.screen.set(layout, status, body);
    }

    fn speak(&mut self, text: String) {
        self.tts.send(TtsRequest::Speak {
            text: text.clone(),
            voice: None,
        });
        self.history.push_front(text);
        self.history.truncate(HISTORY);
    }

    #[cfg(feature = "stt-moshi-xn")]
    fn poll_stt(&mut self) {
        let Some(worker) = &self.stt else {
            return;
        };
        let mut heard = Vec::new();
        for event in worker.poll() {
            match event {
                stt::SttEvent::Loaded { load_ms } => {
                    log::info!("stt: loaded in {load_ms} ms");
                    self.stt_status = format!("{} \u{b7} STT load {load_ms} ms", self.stt_doing);
                }
                stt::SttEvent::LoadFailed(e) => {
                    log::error!("stt: {e}");
                    self.stt_status = format!("STT model missing: {e}");
                }
                stt::SttEvent::Partial(text) => self.partial = text,
                stt::SttEvent::Utterance(text) => {
                    log::info!("stt: heard {text:?}");
                    self.partial.clear();
                    heard.push(text);
                }
                stt::SttEvent::Speed { step_ms, dropped } => {
                    log::info!("stt: step {step_ms:.1} ms, dropped {dropped}");
                    self.stt_status =
                        format!("{} \u{b7} step {step_ms:.0} ms of 80", self.stt_doing);
                    if dropped > 0 {
                        self.stt_status += &format!(" \u{b7} dropped {dropped}");
                    }
                }
                #[cfg(feature = "vad-earshot")]
                stt::SttEvent::Gate(state) => {
                    log::info!("stt: gate {state:?}");
                    self.gate = state;
                    self.stt_doing = match state {
                        vad::GateState::Idle => "tap to talk",
                        vad::GateState::Armed => "listening\u{2026}",
                        vad::GateState::Open => "hearing",
                        vad::GateState::Closing => "finishing",
                    };
                    self.stt_status = self.stt_doing.into();
                }
            }
        }
        for text in heard {
            self.speak(text);
        }
    }
}

impl MorphApp for App {
    /// libmsdf's fragment stage binds five storage buffers; the downlevel
    /// defaults allow four.
    fn limits(base: &wgpu::Limits) -> LimitsRequest {
        let required = wgpu::Limits {
            max_storage_buffers_per_shader_stage: base.max_storage_buffers_per_shader_stage.max(8),
            ..base.clone()
        };
        LimitsRequest {
            preferred: required.clone(),
            required,
        }
    }

    /// With `stt-moshi-xn`, the microphone too, in Android's preset for
    /// speech recognition (plan 20261022_02, decision 8).
    fn audio_setup(setup: &mut AudioSetup) {
        #[cfg(feature = "stt-moshi-xn")]
        {
            setup.capture = true;
            setup.capture_mode = libmorph::CaptureMode::VoiceRecognition;
        }
        #[cfg(not(feature = "stt-moshi-xn"))]
        let _ = setup;
    }

    fn init(ctx: &mut MorphContext) -> Self {
        let atlas = FontAtlas::from_bytes(ROBOTO_REGULAR_ATLAS).expect("bundled atlas parses");
        let shaper = TextShaper::new(ROBOTO_REGULAR_ASCII.to_vec()).expect("bundled face parses");
        let sdf = GpuSdfRenderer::new_with_msdf_layers(
            ctx.device(),
            ctx.surface_format(),
            atlas.width,
            atlas.height,
            atlas.layer_count(),
        );
        sdf.upload_msdf_atlas(
            ctx.queue(),
            atlas.width,
            atlas.height,
            &atlas.to_rgba_bytes(),
        );
        sdf.upload_glyph_table(ctx.queue(), &atlas.glyph_table_u32s());
        // Two seconds of output; the tts thread waits when it is full.
        #[cfg(feature = "stt-moshi-xn")]
        let mut speaking = std::sync::Arc::default();
        let playback = ctx.audio().format().map(|format| {
            let (producer, consumer) = playback::ring(
                format.sample_rate as usize * 2,
                format.channels,
                format.sample_rate,
            );
            #[cfg(feature = "stt-moshi-xn")]
            {
                speaking = consumer.speaking();
            }
            ctx.audio().add_source(morph::source(consumer));
            log::info!(
                "playback: {} Hz, {} channels",
                format.sample_rate,
                format.channels
            );
            producer
        });
        if playback.is_none() {
            log::warn!("playback: no audio output");
        }
        let model = model_dir(ctx, "phonon-31m-en");
        log::info!("tts: loading {}", model.display());
        Self {
            atlas,
            shaper,
            sdf,
            list: DrawList::new(),
            screen: Screen::new(),
            #[cfg(feature = "stt-moshi-xn")]
            next_frame: Instant::now(),
            tts: TtsWorker::spawn(model, playback, None),
            status: "loading\u{2026}".into(),
            history: VecDeque::new(),
            #[cfg(feature = "stt-moshi-xn")]
            stt: None,
            #[cfg(feature = "stt-moshi-xn")]
            stt_status: "waiting for the microphone\u{2026}".into(),
            #[cfg(feature = "stt-moshi-xn")]
            stt_doing: if cfg!(feature = "vad-earshot") {
                "tap to talk"
            } else {
                "listening"
            },
            #[cfg(feature = "vad-earshot")]
            gate: vad::GateState::Idle,
            #[cfg(feature = "stt-moshi-xn")]
            partial: String::new(),
            #[cfg(feature = "stt-moshi-xn")]
            speaking,
        }
    }

    #[cfg(feature = "stt-moshi-xn")]
    fn on_capture_ready(&mut self, ctx: &mut MorphContext) {
        let Some(format) = ctx.audio().capture_format() else {
            return;
        };
        log::info!(
            "capture: {} Hz, {} channels, mode {:?}",
            format.sample_rate,
            format.channels,
            ctx.audio().capture_mode()
        );
        // Four seconds of 24 kHz mono, read by the stt thread every frame.
        let (sink, capture) = morph::capture(format, self.speaking.clone(), 4);
        ctx.audio().add_sink(sink);
        let model = model_dir(ctx, "stt-1b-en_fr");
        log::info!("stt: loading {}", model.display());
        self.stt = Some(stt::SttWorker::spawn(model, capture));
        self.stt_status = "loading STT\u{2026}".into();
    }

    fn update(&mut self, ctx: &mut MorphContext, _dt: Duration) {
        // Hold the frame while STT listens: libmorph renders as fast as
        // presentation allows, and the CPU and GPU heat the phone.
        #[cfg(feature = "stt-moshi-xn")]
        if self.stt.is_some() {
            let now = Instant::now();
            if now < self.next_frame {
                std::thread::sleep(self.next_frame - now);
            }
            let interval = Duration::from_secs(1) / LISTENING_FPS;
            self.next_frame = self.next_frame.max(now) + interval;
        }
        #[cfg(feature = "stt-moshi-xn")]
        self.poll_stt();
        for event in self.tts.poll() {
            log::info!("tts: {event:?}");
            match event {
                TtsEvent::Loaded { voices, load_ms } => {
                    self.status = format!(
                        "TTS ready \u{b7} load {load_ms} ms \u{b7} {} voices",
                        voices.len()
                    );
                }
                TtsEvent::LoadFailed(e) => self.status = format!("model missing: {e}"),
                TtsEvent::FirstAudio { ms, .. } => self.status = format!("first audio {ms} ms"),
                TtsEvent::Finished {
                    audio_ms, synth_ms, ..
                } => {
                    let rtf = f64::from(synth_ms) / f64::from(audio_ms.max(1));
                    self.status = format!("{} \u{b7} RTF {rtf:.2}", self.status);
                }
                TtsEvent::Stopped { .. } => self.status = "stopped".into(),
                TtsEvent::Failed { error, .. } => self.status = format!("failed: {error}"),
                TtsEvent::Started { .. } => {}
            }
        }
        self.lay_out(ctx);
    }

    /// A tap repeats the last utterance, or says the demo sentence; with
    /// `vad-earshot`, once STT has loaded, it is tap to talk.
    fn on_touch(&mut self, _ctx: &mut MorphContext, touch: Touch) {
        // Tap to talk (plan 20261022_03, decision 4): a tap arms, a tap while
        // listening closes, and a tap while voicexn speaks stops it and arms.
        #[cfg(feature = "vad-earshot")]
        if touch.phase == TouchPhase::Ended
            && let Some(stt) = &self.stt
        {
            use std::sync::atomic::Ordering;
            use vad::GateState;
            if self.speaking.load(Ordering::Relaxed) {
                self.tts.send(TtsRequest::Stop);
            }
            match self.gate {
                GateState::Idle => stt.send(stt::SttRequest::Arm),
                GateState::Armed | GateState::Open => stt.send(stt::SttRequest::Close),
                GateState::Closing => {}
            }
            return;
        }
        if touch.phase == TouchPhase::Ended {
            let text = self
                .history
                .front()
                .cloned()
                .unwrap_or_else(|| PHRASE.into());
            self.history.pop_front();
            self.speak(text);
        }
    }

    fn render(&mut self, ctx: &mut MorphContext, frame: &Frame<'_>) {
        let draw = DrawContext {
            px_range: BUNDLED_ATLAS_PX_RANGE,
            ..DrawContext::new(Some(&self.atlas), Some(&self.shaper))
        };
        self.screen.draw(&mut self.list, &draw);
        let (w, h) = frame.size;
        self.sdf.render_draw_list(
            ctx.device(),
            ctx.queue(),
            frame.view,
            w,
            h,
            1.0,
            &self.list,
            0.0,
        );
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: libmorph::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("voicexn"),
    );
    // Before the tts thread starts: rayon's default of one thread per core
    // is too many on phones that mix fast and slow cores (plan section 4).
    // VOICEXN_THREADS at build time overrides it, to compare thread counts.
    // STT needs 4 on the S23 to keep up with 80 ms frames.
    let default = if cfg!(feature = "stt-moshi-xn") {
        "4"
    } else {
        "2"
    };
    let threads = option_env!("VOICEXN_THREADS").unwrap_or(default);
    // SAFETY: no other thread has started yet.
    unsafe { std::env::set_var("RAYON_NUM_THREADS", threads) };
    // VOICEXN_STT_QUANT at build time picks the STT model's quantization.
    #[cfg(feature = "stt-moshi-xn")]
    if let Some(quant) = option_env!("VOICEXN_STT_QUANT") {
        // SAFETY: as above.
        unsafe { std::env::set_var("VOICEXN_STT_QUANT", quant) };
    }
    libmorph::run_android::<App>(app);
}
