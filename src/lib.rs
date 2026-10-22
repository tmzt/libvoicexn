//! libvoicexn: speech in and out on xn, without ONNX
//! (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md). Phonon
//! text-to-speech (docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md),
//! and with `stt-moshi-xn`, Kyutai speech-to-text on xn-moshi
//! (docs/design/20261022_02_STT_MOSHI_XN_PLAN.md). `examples/voicexn` is
//! the libmorph demo.

#[cfg(feature = "stt-moshi-xn-boost")]
pub mod boost;
#[cfg(feature = "stt-moshi-xn")]
pub mod capture;
#[cfg(feature = "morph")]
pub mod morph;
pub mod playback;
#[cfg(feature = "stt-moshi-xn")]
pub mod spm;
#[cfg(feature = "stt-moshi-xn")]
pub mod stt;
pub mod tts;
#[cfg(feature = "vad-earshot")]
pub mod vad;
