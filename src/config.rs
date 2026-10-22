//! What the workers load, in place of build-time variables
//! (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md, decision 5). The
//! `VOICEXN_*` overrides are read here and nowhere else in the library.

use std::path::PathBuf;

/// The STT LM's quantization, and its file in the model folder.
#[cfg(feature = "stt-moshi-xn")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttQuant {
    /// `model.q4k.gguf`: the one that keeps up on the S23 (CHANGES.md,
    /// plan 20261022_02).
    Q4k,
    /// `model.q4_0.gguf`.
    Q4_0,
    /// `model.q8.gguf`.
    Q8_0,
}

#[cfg(feature = "stt-moshi-xn")]
impl SttQuant {
    /// Android's: q4k keeps the step inside the 80 ms frame on the S23.
    pub const ANDROID: Self = Self::Q4k;
    /// The desktop's: q4k too, so the desktop runs what the phone runs
    /// (plan 20261022_05, step 4; CHANGES.md).
    pub const DESKTOP: Self = Self::Q4k;

    /// `q4k`, `q4_0` or `q8_0` (also `q8`), as `VOICEXN_STT_QUANT` and
    /// `--quant` take it.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "q4k" => Some(Self::Q4k),
            "q4_0" => Some(Self::Q4_0),
            "q8_0" | "q8" => Some(Self::Q8_0),
            _ => None,
        }
    }

    /// The platform's default: [`ANDROID`](Self::ANDROID) or
    /// [`DESKTOP`](Self::DESKTOP).
    pub fn for_platform() -> Self {
        if cfg!(target_os = "android") {
            Self::ANDROID
        } else {
            Self::DESKTOP
        }
    }

    /// The GGUF in the model folder.
    pub fn file(self) -> &'static str {
        match self {
            Self::Q4k => "model.q4k.gguf",
            Self::Q4_0 => "model.q4_0.gguf",
            Self::Q8_0 => "model.q8.gguf",
        }
    }
}

/// Kyutai STT: the model folder, the LM's quantization and, with
/// `stt-moshi-xn-boost`, the phrase list and its biases.
#[cfg(feature = "stt-moshi-xn")]
#[derive(Clone, Debug)]
pub struct SttConfig {
    pub model_dir: PathBuf,
    pub quant: SttQuant,
    /// One phrase per line; `None` boosts nothing.
    #[cfg(feature = "stt-moshi-xn-boost")]
    pub vocab: Option<PathBuf>,
    /// `start` and `cont` (plan 20261022_04, decision 4).
    #[cfg(feature = "stt-moshi-xn-boost")]
    pub boost: (f32, f32),
}

#[cfg(feature = "stt-moshi-xn")]
impl SttConfig {
    /// The platform's quantization, then `VOICEXN_STT_QUANT`,
    /// `VOICEXN_VOCAB` and `VOICEXN_BOOST=start,cont` if set. Without
    /// `VOICEXN_VOCAB` the list is `vocab.txt` in the working directory
    /// (the desktop), else `vocab.txt` next to the model folder (Android's
    /// data directory), if either exists.
    pub fn for_platform(model_dir: impl Into<PathBuf>) -> Self {
        Self::with_env(model_dir.into(), |name| std::env::var(name).ok())
    }

    fn with_env(model_dir: PathBuf, var: impl Fn(&str) -> Option<String>) -> Self {
        let quant = match var("VOICEXN_STT_QUANT") {
            Some(name) => SttQuant::parse(&name).unwrap_or_else(|| {
                log::warn!("stt: unknown VOICEXN_STT_QUANT {name:?}");
                SttQuant::for_platform()
            }),
            None => SttQuant::for_platform(),
        };
        #[cfg(feature = "stt-moshi-xn-boost")]
        let vocab = var("VOICEXN_VOCAB").map(PathBuf::from).or_else(|| {
            [
                Some(PathBuf::from("vocab.txt")),
                model_dir.parent().map(|p| p.join("vocab.txt")),
            ]
            .into_iter()
            .flatten()
            .find(|p| p.is_file())
        });
        #[cfg(feature = "stt-moshi-xn-boost")]
        let boost = var("VOICEXN_BOOST")
            .and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .unwrap_or((crate::boost::START, crate::boost::CONT));
        Self {
            model_dir,
            quant,
            #[cfg(feature = "stt-moshi-xn-boost")]
            vocab,
            #[cfg(feature = "stt-moshi-xn-boost")]
            boost,
        }
    }
}

/// Phonon: the checkpoint folder, its quantization, and where to write
/// each utterance as `<record>/<unix ms>.wav`, if anywhere.
#[derive(Clone, Debug)]
pub struct TtsConfig {
    pub model_dir: PathBuf,
    pub quant: ptts::synth::Quant,
    pub record: Option<PathBuf>,
}

impl TtsConfig {
    /// q8 on every platform, as plan 20261022_01 chose; nothing recorded.
    pub fn for_platform(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            quant: ptts::synth::Quant::Q80,
            record: None,
        }
    }
}

#[cfg(all(test, feature = "stt-moshi-xn"))]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_platform_default_without_overrides() {
        let c = SttConfig::with_env("/nowhere/stt".into(), env(&[]));
        assert_eq!(c.model_dir, PathBuf::from("/nowhere/stt"));
        assert_eq!(c.quant, SttQuant::Q4k);
        #[cfg(feature = "stt-moshi-xn-boost")]
        assert_eq!(c.boost, (crate::boost::START, crate::boost::CONT));
    }

    #[test]
    fn the_overrides_win() {
        let c = SttConfig::with_env(
            "/nowhere/stt".into(),
            env(&[
                ("VOICEXN_STT_QUANT", "q4_0"),
                ("VOICEXN_VOCAB", "/nowhere/words.txt"),
                ("VOICEXN_BOOST", "2, 16"),
            ]),
        );
        assert_eq!(c.quant, SttQuant::Q4_0);
        #[cfg(feature = "stt-moshi-xn-boost")]
        {
            assert_eq!(c.vocab, Some(PathBuf::from("/nowhere/words.txt")));
            assert_eq!(c.boost, (2.0, 16.0));
        }
    }

    #[test]
    fn a_bad_override_keeps_the_default() {
        let c = SttConfig::with_env(
            "/nowhere/stt".into(),
            env(&[("VOICEXN_STT_QUANT", "q3"), ("VOICEXN_BOOST", "lots")]),
        );
        assert_eq!(c.quant, SttQuant::for_platform());
        #[cfg(feature = "stt-moshi-xn-boost")]
        assert_eq!(c.boost, (crate::boost::START, crate::boost::CONT));
    }

    #[test]
    fn quant_names_and_files() {
        assert_eq!(SttQuant::parse("q8"), Some(SttQuant::Q8_0));
        assert_eq!(
            SttQuant::parse("q8_0").map(SttQuant::file),
            Some("model.q8.gguf")
        );
        assert_eq!(
            SttQuant::parse("q4k").map(SttQuant::file),
            Some("model.q4k.gguf")
        );
        assert_eq!(
            SttQuant::parse("q4_0").map(SttQuant::file),
            Some("model.q4_0.gguf")
        );
        assert_eq!(SttQuant::parse("f32"), None);
    }
}
