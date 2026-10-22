//! voicexn: Phonon text-to-speech on xn, in a libmorph app
//! (docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md).

pub mod tts;

use libcontrols::{DrawContext, DrawControl, Label, Rect, TextAlign};
use libmorph::{Frame, LimitsRequest, MorphApp, MorphContext, wgpu};
use libmsdf::GpuSdfRenderer;
use libmsdf::drawlist::DrawList;
use libmsdf::font::{
    BUNDLED_ATLAS_PX_RANGE, FontAtlas, ROBOTO_REGULAR_ASCII, ROBOTO_REGULAR_ATLAS, TextShaper,
};

const TEXT: [f32; 4] = [0.92, 0.93, 0.95, 1.0];

pub struct App {
    atlas: FontAtlas,
    shaper: TextShaper,
    sdf: GpuSdfRenderer,
    list: DrawList,
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
        Self {
            atlas,
            shaper,
            sdf,
            list: DrawList::new(),
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
    libmorph::run_android::<App>(app);
}
