//! libmorph adapters, behind `morph`
//! (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md, decision 4): the
//! core's audio ends as a libmorph `Sink` and `Source`.

#[cfg(feature = "stt-moshi-xn")]
use std::sync::Arc;
#[cfg(feature = "stt-moshi-xn")]
use std::sync::atomic::AtomicBool;

use libmorph::Source;
#[cfg(feature = "stt-moshi-xn")]
use libmorph::{CaptureFormat, Captured, Sink};

#[cfg(feature = "stt-moshi-xn")]
use crate::capture::Mic;
use crate::playback::Consumer;
#[cfg(feature = "stt-moshi-xn")]
use crate::stt::Capture;

/// The libmorph `Sink` that feeds `mic`.
#[cfg(feature = "stt-moshi-xn")]
pub fn sink(mut mic: Mic) -> Sink {
    Box::new(move |captured: &Captured<'_>| mic.push(captured.samples))
}

/// The libmorph `Source` that plays the ring.
pub fn source(mut consumer: Consumer) -> Source {
    Box::new(move |out: &mut [f32]| consumer.fill(out))
}

/// The sink for `Audio::add_sink` in `format`, and the ring it fills
/// (`seconds` long); see [`Mic::new`].
#[cfg(feature = "stt-moshi-xn")]
pub fn capture(format: CaptureFormat, speaking: Arc<AtomicBool>, seconds: u32) -> (Sink, Capture) {
    let (mic, capture) = Mic::new(format.sample_rate, format.channels, speaking, seconds);
    (sink(mic), capture)
}
