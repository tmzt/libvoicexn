//! From the `tts` thread to the speaker (plan section 2, decision 4): a
//! single-producer, single-consumer ring of interleaved `f32` frames at the
//! output's rate, a resampler from Phonon's 24 kHz, and the consumer that
//! pops from the ring on the audio thread without blocking or allocating.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// The audio thread's end: [`fill`](Self::fill) it from the output
/// callback, or wrap it in a [`Playback`] for a flush at any time and a cue.
pub struct Consumer {
    ring: rtrb::Consumer<f32>,
    flush: Arc<AtomicBool>,
    speaking: Arc<AtomicBool>,
    channels: usize,
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
    let consumer = Consumer {
        ring: rx,
        flush,
        speaking: Arc::new(AtomicBool::new(false)),
        channels,
    };
    (producer, consumer)
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
    /// audio thread does the dropping; this waits for it, up to 100 ms.
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
    /// True while the last fill played something: voicexn is speaking.
    pub fn speaking(&self) -> Arc<AtomicBool> {
        self.speaking.clone()
    }

    /// Drops everything queued in the ring, on the audio thread.
    fn drain(&mut self) {
        let n = self.ring.slots();
        if let Ok(chunk) = self.ring.read_chunk(n) {
            chunk.commit_all();
        }
        self.speaking.store(false, Ordering::Relaxed);
    }

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
        self.speaking.store(n > 0, Ordering::Relaxed);
    }
}

/// The chime's length, in seconds.
const CHIME_SECONDS: f32 = 0.35;

/// A short chime at `rate`, mono: two bell partials (C6 and G6) that start
/// over 5 ms and die away, at a quarter of full scale. [`Playback`]'s
/// default cue in the session.
pub fn chime(rate: u32) -> Vec<f32> {
    let rate = rate.max(1) as f32;
    let len = (CHIME_SECONDS * rate) as usize;
    (0..len)
        .map(|i| {
            let t = i as f32 / rate;
            let attack = (t / 0.005).min(1.0);
            let decay = (-t / 0.09).exp();
            let tau = std::f32::consts::TAU;
            let tone = (tau * 1046.5 * t).sin() + 0.5 * (tau * 1568.0 * t).sin();
            0.25 * attack * decay * tone / 1.5
        })
        .collect()
}

/// The output callback's end for an app: a [`Consumer`] that can also be
/// flushed at any time and plays a cue over what the ring gives
/// (docs/design/20261029_01_SESSION_PLAN.md, decision 4).
///
/// The TTS worker flushes the ring itself only when a `Stop` reaches it
/// while it is still making an utterance. Phonon runs ahead of the speaker,
/// so a reply is often fully made, with up to the ring's length still to
/// play, when the app stops it. `Playback` drains the ring whenever its
/// flush count moves. [`fill`](Self::fill) never blocks or allocates.
pub struct Playback {
    consumer: Consumer,
    flush: Arc<AtomicU32>,
    seen: u32,
    /// The cue, mono at the output's rate, spread over its channels.
    cue: Vec<f32>,
    cues: Arc<AtomicU32>,
    cues_seen: u32,
    cue_at: Option<usize>,
}

impl Playback {
    /// Wraps `consumer`; the ring is drained each time `flush` moves.
    pub fn new(consumer: Consumer, flush: Arc<AtomicU32>) -> Self {
        let seen = flush.load(Ordering::Acquire);
        Self {
            consumer,
            flush,
            seen,
            cue: Vec::new(),
            cues: Arc::new(AtomicU32::new(0)),
            cues_seen: 0,
            cue_at: None,
        }
    }

    /// Mixes `cue` (mono, at the output's rate) in from its start each time
    /// `cues` moves, on every channel.
    pub fn with_cue(mut self, cue: Vec<f32>, cues: Arc<AtomicU32>) -> Self {
        self.cue = cue;
        self.cues_seen = cues.load(Ordering::Acquire);
        self.cues = cues;
        self
    }

    /// True while the last fill played speech from the ring.
    pub fn speaking(&self) -> Arc<AtomicBool> {
        self.consumer.speaking()
    }

    /// Fills `out` from the ring, after dropping what was queued if the
    /// flush count moved; what the ring does not cover is zeroed.
    pub fn fill(&mut self, out: &mut [f32]) {
        let want = self.flush.load(Ordering::Acquire);
        if want != self.seen {
            self.seen = want;
            self.consumer.drain();
        }
        // `Consumer::fill` writes only what the ring holds; a device buffer
        // is not always zeroed for us.
        out.fill(0.0);
        self.consumer.fill(out);
        self.mix_cue(out);
    }

    fn mix_cue(&mut self, out: &mut [f32]) {
        let want = self.cues.load(Ordering::Acquire);
        if want != self.cues_seen {
            self.cues_seen = want;
            self.cue_at = Some(0);
        }
        let Some(mut at) = self.cue_at else {
            return;
        };
        for frame in out.chunks_mut(self.consumer.channels) {
            let Some(&sample) = self.cue.get(at) else {
                break;
            };
            for s in frame {
                *s = (*s + sample).clamp(-1.0, 1.0);
            }
            at += 1;
        }
        self.cue_at = (at < self.cue.len()).then_some(at);
    }
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

    #[test]
    fn a_flush_drops_what_is_queued_and_stops_playing() {
        let (mut tx, rx) = ring(20_000, 1, TTS_RATE);
        let flush = Arc::new(AtomicU32::new(0));
        let mut playback = Playback::new(rx, flush.clone());
        let playing = playback.speaking();
        assert!(tx.push(&[0.5; 12_000], || false));
        let mut out = [0.0; 8];
        playback.fill(&mut out);
        assert_eq!(out, [0.5; 8]);
        assert!(playing.load(Ordering::Relaxed));

        flush.fetch_add(1, Ordering::Release);
        let mut out = [9.0; 8];
        playback.fill(&mut out);
        assert_eq!(out, [0.0; 8], "nothing left: silence, zeroed");
        assert!(!playing.load(Ordering::Relaxed));

        // New speech after the flush plays.
        tx.reset();
        assert!(tx.push(&[0.25; 100], || false));
        let mut out = [0.0; 4];
        playback.fill(&mut out);
        assert_eq!(out, [0.25; 4]);
    }

    #[test]
    fn the_tail_past_the_ring_is_zeroed() {
        let (mut tx, rx) = ring(1000, 1, TTS_RATE);
        let mut playback = Playback::new(rx, Arc::new(AtomicU32::new(0)));
        assert!(tx.push(&[0.5; 4], || false));
        let mut out = [9.0; 64];
        playback.fill(&mut out);
        let played = out.iter().take_while(|&&s| s != 0.0).count();
        assert!(played > 0 && played < 64, "{out:?}");
        assert!(out[played..].iter().all(|&s| s == 0.0), "{out:?}");
    }

    #[test]
    fn the_chime_is_short_and_quiet() {
        let c = chime(48_000);
        assert_eq!(c.len(), (CHIME_SECONDS * 48_000.0) as usize);
        assert!(c.iter().all(|s| s.abs() <= 0.25));
        assert!(c.iter().any(|s| s.abs() > 0.1), "it is heard");
        assert_eq!(c[0], 0.0, "it starts from silence");
    }

    #[test]
    fn a_cue_plays_once_on_both_channels_over_the_ring() {
        let (mut tx, rx) = ring(20_000, 2, TTS_RATE);
        let cues = Arc::new(AtomicU32::new(0));
        let mut playback =
            Playback::new(rx, Arc::new(AtomicU32::new(0))).with_cue(chime(48_000), cues.clone());
        let mut out = vec![0.0; 2000];
        playback.fill(&mut out);
        assert!(out.iter().all(|&s| s == 0.0), "nothing asked: silence");

        cues.fetch_add(1, Ordering::Release);
        let reference = chime(48_000);
        let mut played = Vec::new();
        while played.len() <= reference.len() + 1000 {
            let mut out = vec![0.0; 2000];
            playback.fill(&mut out);
            for frame in out.chunks(2) {
                assert_eq!(frame[0], frame[1], "the same on both channels");
                played.push(frame[0]);
            }
        }
        assert_eq!(&played[..reference.len()], &reference[..]);
        assert!(played[reference.len()..].iter().all(|&s| s == 0.0), "once");

        // Over speech, it is added to it.
        assert!(tx.push(&[0.1; 4000], || false));
        cues.fetch_add(1, Ordering::Release);
        let mut out = vec![0.0; 200];
        playback.fill(&mut out);
        let speech: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        assert!(
            speech
                .iter()
                .zip(&reference)
                .any(|(s, c)| *c != 0.0 && *s != *c)
        );
    }
}
