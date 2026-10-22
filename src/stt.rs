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
