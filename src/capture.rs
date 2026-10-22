//! The microphone end of speech-to-text
//! (docs/design/20261022_02_STT_MOSHI_XN_PLAN.md, decisions 5 and 6;
//! docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md, decision 4): [`Mic`]
//! turns interleaved input into 24 kHz mono for Mimi and pushes it into a
//! ring, without blocking the input thread. While voicexn speaks it pushes
//! nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::playback::Resampler;
use crate::stt::{Capture, CaptureStats, ECHO_HOLD_S, STT_RATE};

/// The writing end of the capture, for the input device's callback.
pub struct Mic {
    ring: rtrb::Producer<f32>,
    stats: Arc<CaptureStats>,
    channels: usize,
    resampler: Resampler,
    mono: Vec<f32>,
    out: Vec<f32>,
    speaking: Arc<AtomicBool>,
    /// Input frames muted after speaking stops.
    hold: usize,
    /// Input frames still to mute after speaking stopped.
    muted: usize,
}

impl Mic {
    /// A microphone at `in_rate` with `channels` interleaved channels, and
    /// the ring it fills (`seconds` long). While `speaking` is set, and for
    /// [`ECHO_HOLD_S`] after, [`push`](Self::push) pushes nothing: the model
    /// does not hear voicexn, and pauses instead of stepping through
    /// silence while Phonon has the CPU.
    pub fn new(
        in_rate: u32,
        channels: u16,
        speaking: Arc<AtomicBool>,
        seconds: u32,
    ) -> (Mic, Capture) {
        let (ring, consumer) = rtrb::RingBuffer::new((STT_RATE * seconds) as usize);
        let stats = Arc::new(CaptureStats::default());
        let mic = Mic {
            ring,
            stats: stats.clone(),
            channels: usize::from(channels.max(1)),
            resampler: Resampler::new(in_rate, STT_RATE),
            // Room for blocks of up to a second; larger ones grow it once.
            mono: Vec::with_capacity(in_rate as usize),
            out: Vec::with_capacity(STT_RATE as usize),
            speaking,
            hold: (ECHO_HOLD_S * in_rate as f32) as usize,
            muted: 0,
        };
        (
            mic,
            Capture {
                ring: consumer,
                stats,
            },
        )
    }

    /// Downmixes, measures, resamples and pushes one block of interleaved
    /// samples. Samples that do not fit in the ring are counted as dropped.
    pub fn push(&mut self, interleaved: &[f32]) {
        self.mono.clear();
        let mut peak = 0.0f32;
        for frame in interleaved.chunks_exact(self.channels) {
            let x = frame.iter().sum::<f32>() / self.channels as f32;
            peak = peak.max(x.abs());
            self.mono.push(x);
        }
        self.stats.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        if self.speaking.load(Ordering::Relaxed) {
            self.muted = self.hold;
            return;
        }
        if self.muted > 0 {
            let skip = self.muted.min(self.mono.len());
            self.muted -= skip;
            self.mono.drain(..skip);
        }
        self.out.clear();
        self.resampler.process(&self.mono, &mut self.out);
        let n = self.out.len().min(self.ring.slots());
        if let Ok(mut chunk) = self.ring.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            for (slot, &x) in a.iter_mut().chain(b.iter_mut()).zip(&self.out) {
                slot.write(x);
            }
            // SAFETY: all `n` slots were written above.
            unsafe { chunk.commit_all() };
        }
        if n < self.out.len() {
            self.stats
                .dropped
                .fetch_add((self.out.len() - n) as u64, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_48k_arrives_as_mono_24k_and_nothing_while_speaking() {
        let speaking = Arc::new(AtomicBool::new(false));
        let (mut mic, mut capture) = Mic::new(48_000, 2, speaking.clone(), 1);
        // Left 0.5, right -0.1: mono 0.2.
        let stereo: Vec<f32> = [0.5, -0.1].repeat(480);
        mic.push(&stereo);
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
        mic.push(&stereo);
        assert_eq!(capture.ring.slots(), 0);
        // The level is still measured: it is the microphone, not the ring.
        assert_eq!(capture.stats.take().0, 0.2);

        // Then the echo hold: 0.3 s of 48 kHz is 14 400 frames, 30 blocks
        // of 480 muted, and the 31st heard.
        speaking.store(false, Ordering::Relaxed);
        for _ in 0..30 {
            mic.push(&stereo);
        }
        assert_eq!(capture.ring.slots(), 0);
        mic.push(&stereo);
        assert!(capture.ring.slots() > 200);
    }

    #[test]
    fn a_full_ring_drops_and_counts() {
        let (mut mic, capture) = Mic::new(24_000, 1, Arc::new(AtomicBool::new(false)), 1);
        let second = vec![0.0; 24_000];
        mic.push(&second);
        mic.push(&second);
        assert!(capture.stats.take().1 >= 23_999);
    }
}
