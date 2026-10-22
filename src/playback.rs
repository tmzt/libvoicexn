//! From the `tts` thread to the speaker (plan section 2, decision 4): a
//! single-producer, single-consumer ring of interleaved `f32` frames at the
//! output's rate, a resampler from Phonon's 24 kHz, and the libmorph
//! `Source` that pops from the ring on the audio thread without blocking or
//! allocating.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Phonon's output rate.
pub const TTS_RATE: u32 = 24_000;

/// How long the producer waits for the audio thread to flush on `Stop`
/// before going on; an output that is paused never answers.
const FLUSH_WAIT: Duration = Duration::from_millis(100);

/// The `tts` thread's end: resamples mono 24 kHz chunks and pushes them.
pub struct Producer {
    ring: rtrb::Producer<f32>,
    flush: Arc<AtomicBool>,
    channels: usize,
    resampler: Resampler,
    scratch: Vec<f32>,
}

/// The audio thread's end, turned into a libmorph `Source` by [`source`].
pub struct Consumer {
    ring: rtrb::Consumer<f32>,
    flush: Arc<AtomicBool>,
}

/// A ring holding `capacity_frames` frames of `channels` interleaved samples
/// at `out_rate`.
pub fn ring(capacity_frames: usize, channels: u16, out_rate: u32) -> (Producer, Consumer) {
    let channels = usize::from(channels.max(1));
    let (tx, rx) = rtrb::RingBuffer::new(capacity_frames * channels);
    let flush = Arc::new(AtomicBool::new(false));
    let producer = Producer {
        ring: tx,
        flush: flush.clone(),
        channels,
        resampler: Resampler::new(TTS_RATE, out_rate),
        scratch: Vec::new(),
    };
    (producer, Consumer { ring: rx, flush })
}

impl Producer {
    /// Resamples `pcm` (mono, 24 kHz) and pushes it, waiting for space while
    /// the ring is full. Returns `false` if `cancelled` says so while it
    /// waits.
    pub fn push(&mut self, pcm: &[f32], mut cancelled: impl FnMut() -> bool) -> bool {
        let Self {
            ring,
            channels,
            resampler,
            scratch,
            ..
        } = self;
        scratch.clear();
        resampler.process(pcm, scratch);
        let mut done = 0;
        loop {
            let n = (ring.slots() / *channels).min(scratch.len() - done);
            for &sample in &scratch[done..done + n] {
                for _ in 0..*channels {
                    // There is room: `slots` counted it.
                    let _ = ring.push(sample);
                }
            }
            done += n;
            if done == scratch.len() {
                return true;
            }
            if cancelled() {
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Drops what is queued and what the resampler holds, for `Stop`. The
    /// audio thread does the dropping; this waits for it, up to
    /// [`FLUSH_WAIT`].
    pub fn flush(&mut self) {
        self.resampler.reset();
        self.flush.store(true, Ordering::Release);
        let start = Instant::now();
        while self.flush.load(Ordering::Acquire) && start.elapsed() < FLUSH_WAIT {
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Starts a new utterance: the resampler forgets the last one.
    pub fn reset(&mut self) {
        self.resampler.reset();
    }
}

impl Consumer {
    /// Fills `out` from the ring, leaving zeros where it runs dry.
    pub fn fill(&mut self, out: &mut [f32]) {
        if self.flush.load(Ordering::Acquire) {
            let n = self.ring.slots();
            if let Ok(chunk) = self.ring.read_chunk(n) {
                chunk.commit_all();
            }
            self.flush.store(false, Ordering::Release);
        }
        let n = out.len().min(self.ring.slots());
        if let Ok(chunk) = self.ring.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
    }
}

/// The libmorph `Source` that plays the ring.
pub fn source(mut consumer: Consumer) -> libmorph::Source {
    Box::new(move |out: &mut [f32]| consumer.fill(out))
}

/// Linear-interpolating resampler for one channel that carries its position
/// and last sample across chunks, so chunk boundaries leave no seam.
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output sample, in input samples, relative to
    /// `prev` at -1 and the chunk's first sample at 0.
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            step: f64::from(in_rate) / f64::from(out_rate.max(1)),
            pos: 0.0,
            prev: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.prev = 0.0;
    }

    /// Appends the resampled `input` to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let Some(&last) = input.last() else {
            return;
        };
        let at = |i: isize| if i < 0 { self.prev } else { input[i as usize] };
        let len = input.len() as f64;
        // Interpolate between floor(pos) and floor(pos) + 1, both of which
        // must be in [-1, len - 1]. A position on the last sample waits for
        // the next chunk, where it is -1, `prev`.
        while self.pos < len - 1.0 {
            let i = self.pos.floor();
            let t = (self.pos - i) as f32;
            let (a, b) = (at(i as isize), at(i as isize + 1));
            out.push(a + (b - a) * t);
            self.pos += self.step;
        }
        self.pos -= len;
        self.prev = last;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn a_24k_sine_keeps_its_frequency_at_48k() {
        let input = sine(440.0, TTS_RATE, TTS_RATE as usize);
        let mut out = Vec::new();
        Resampler::new(TTS_RATE, 48_000).process(&input, &mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        // One second of 440 Hz rises through zero 440 times, give or take one.
        assert!(
            (crossings(&out) as i64 - 440).abs() <= 1,
            "{}",
            crossings(&out)
        );
    }

    #[test]
    fn chunk_boundaries_leave_no_seam() {
        let input = sine(440.0, TTS_RATE, 9_600);
        for out_rate in [44_100, 48_000] {
            let mut whole = Vec::new();
            Resampler::new(TTS_RATE, out_rate).process(&input, &mut whole);
            let mut chunked = Vec::new();
            let mut r = Resampler::new(TTS_RATE, out_rate);
            // 80 ms frames, as Phonon makes them, and an odd size.
            for chunk in input.chunks(1_920).flat_map(|c| c.chunks(777)) {
                r.process(chunk, &mut chunked);
            }
            assert_eq!(whole.len(), chunked.len(), "{out_rate}");
            let worst = whole
                .iter()
                .zip(&chunked)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-5, "{out_rate}: {worst}");
        }
    }

    #[test]
    fn the_ring_plays_in_order_on_every_channel_and_flushes() {
        let (mut tx, mut rx) = ring(1_000, 2, TTS_RATE);
        // The last sample of a chunk is held for the next one.
        assert!(tx.push(&[0.1, 0.2, 0.3, 0.4], || false));
        let mut out = [9.0; 8];
        rx.fill(&mut out);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 9.0, 9.0]);
        assert!(tx.push(&[0.5; 10], || false));
        tx.flush.store(true, Ordering::Release);
        let mut out = [0.0; 4];
        rx.fill(&mut out);
        assert_eq!(out, [0.0; 4]);
        assert!(!tx.flush.load(Ordering::Acquire));
    }

    #[test]
    fn a_full_ring_gives_up_when_cancelled() {
        let (mut tx, _rx) = ring(4, 1, TTS_RATE);
        assert!(!tx.push(&[0.0; 10], || true));
    }
}
