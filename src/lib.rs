//! voicexn: Phonon text-to-speech on xn, in a libmorph app
//! (docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md).

pub mod playback;
pub mod tts;

use std::path::PathBuf;
use std::time::Duration;

use libcontrols::{DrawContext, DrawControl, Label, Rect, TextAlign};
use libmorph::{Frame, LimitsRequest, MorphApp, MorphContext, Touch, TouchPhase, wgpu};
use libmsdf::GpuSdfRenderer;
use libmsdf::drawlist::DrawList;
use libmsdf::font::{
    BUNDLED_ATLAS_PX_RANGE, FontAtlas, ROBOTO_REGULAR_ASCII, ROBOTO_REGULAR_ATLAS, TextShaper,
};

use crate::tts::{TtsEvent, TtsRequest, TtsWorker};

const TEXT: [f32; 4] = [0.92, 0.93, 0.95, 1.0];
const MUTED: [f32; 4] = [0.60, 0.62, 0.68, 1.0];

/// Spoken on a tap until the screen of step 4 has phrases.
const PHRASE: &str =
    "Phonon turns text into natural speech, right on your device, with no cloud round trip.";

/// The checkpoint folder: `models/phonon-31m-en` on the desktop (or
/// `--model <dir>`), `<data dir>/phonon-31m-en` on Android.
fn model_dir(ctx: &MorphContext) -> PathBuf {
    if let Some(dir) = ctx.data_dir() {
        return dir.join("phonon-31m-en");
    }
    let args = ctx.args();
    args.iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| PathBuf::from("models/phonon-31m-en"), PathBuf::from)
}

pub struct App {
    atlas: FontAtlas,
    shaper: TextShaper,
    sdf: GpuSdfRenderer,
    list: DrawList,
    tts: TtsWorker,
    status: String,
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
        let playback = ctx.audio().format().map(|format| {
            let (producer, consumer) = playback::ring(
                format.sample_rate as usize * 2,
                format.channels,
                format.sample_rate,
            );
            ctx.audio().add_source(playback::source(consumer));
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
        let model = model_dir(ctx);
        log::info!("tts: loading {}", model.display());
        Self {
            atlas,
            shaper,
            sdf,
            list: DrawList::new(),
            tts: TtsWorker::spawn(model, playback, None),
            status: "loading\u{2026}".into(),
        }
    }

    fn update(&mut self, _ctx: &mut MorphContext, _dt: Duration) {
        for event in self.tts.poll() {
            log::info!("tts: {event:?}");
            match event {
                TtsEvent::Loaded { voices, load_ms } => {
                    self.status = format!(
                        "ready \u{b7} load {load_ms} ms \u{b7} {} voices",
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
    }

    fn on_touch(&mut self, _ctx: &mut MorphContext, touch: Touch) {
        if touch.phase == TouchPhase::Ended {
            self.tts.send(TtsRequest::Speak {
                text: PHRASE.into(),
                voice: None,
            });
        }
    }

    fn render(&mut self, ctx: &mut MorphContext, frame: &Frame<'_>) {
        let (w, h) = frame.size;
        let s = ctx.scale_factor();
        let draw = DrawContext {
            px_range: BUNDLED_ATLAS_PX_RANGE,
            ..DrawContext::new(Some(&self.atlas), Some(&self.shaper))
        };
        self.list.clear();
        Label::new(
            "voicexn",
            Rect::new(0.0, h as f32 * 0.4, w as f32, 48.0 * s),
        )
        .with_font_size(40.0 * s)
        .with_color(TEXT)
        .with_align(TextAlign::Center)
        .draw(&mut self.list, &draw);
        Label::new(
            self.status.as_str(),
            Rect::new(0.0, h as f32 * 0.4 + 64.0 * s, w as f32, 32.0 * s),
        )
        .with_font_size(18.0 * s)
        .with_color(MUTED)
        .with_align(TextAlign::Center)
        .draw(&mut self.list, &draw);
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
    // SAFETY: no other thread has started yet.
    unsafe { std::env::set_var("RAYON_NUM_THREADS", "2") };
    libmorph::run_android::<App>(app);
}
