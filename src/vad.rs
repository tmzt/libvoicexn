//! Tap to talk (docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md): the gate
//! that decides which audio reaches the STT model, driven by a voice score
//! per 16 ms frame. This module holds no detector; the `stt` thread scores
//! frames with earshot and hands the scores in.

use std::collections::VecDeque;

/// One earshot frame, 16 ms, in 24 kHz samples.
pub const SUB: usize = 384;

/// A score at or above this is voice.
pub const VOICE: f32 = 0.5;
/// A score below this is quiet.
pub const QUIET: f32 = 0.35;
/// Voice frames in a row that open the gate: 80 ms.
pub const OPEN_RUN: u32 = 5;
/// Quiet frames in a row that start closing it: 800 ms.
pub const CLOSE_RUN: u32 = 50;
/// Frames armed with no voice before giving up: 6 s.
pub const ARM_TIMEOUT: u32 = 375;
/// Frames of closing before the gate is idle even with no utterance: 3 s.
pub const CLOSING_MAX: u32 = 188;
/// Audio kept while armed and fed in when the gate opens: 320 ms.
pub const PRE_ROLL: usize = 20 * SUB;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateState {
    /// Nothing is scored or transcribed.
    Idle,
    /// Waiting for voice after a tap.
    Armed,
    /// Voice: the audio goes to STT.
    Open,
    /// Voice has stopped: STT gets silence until the utterance ends.
    Closing,
}

pub struct Gate {
    state: GateState,
    /// Frames spent in the current state.
    frames: u32,
    voice_run: u32,
    quiet_run: u32,
    pre_roll: VecDeque<f32>,
}

impl Gate {
    pub fn new() -> Self {
        Self {
            state: GateState::Idle,
            frames: 0,
            voice_run: 0,
            quiet_run: 0,
            pre_roll: VecDeque::with_capacity(PRE_ROLL),
        }
    }

    pub fn state(&self) -> GateState {
        self.state
    }

    /// Whether frames need a score: everywhere but `Idle`.
    pub fn listening(&self) -> bool {
        self.state != GateState::Idle
    }

    fn enter(&mut self, state: GateState) -> Option<GateState> {
        self.state = state;
        self.frames = 0;
        self.voice_run = 0;
        self.quiet_run = 0;
        if state != GateState::Armed {
            self.pre_roll.clear();
        }
        Some(state)
    }

    /// A tap in `Idle`: listen.
    pub fn arm(&mut self) -> Option<GateState> {
        (self.state == GateState::Idle).then(|| self.enter(GateState::Armed))?
    }

    /// A tap while listening: stop. With nothing heard, back to `Idle`;
    /// with the gate open, finish the utterance.
    pub fn close(&mut self) -> Option<GateState> {
        match self.state {
            GateState::Armed => self.enter(GateState::Idle),
            GateState::Open => self.enter(GateState::Closing),
            GateState::Idle | GateState::Closing => None,
        }
    }

    /// The utterance has ended: back to `Idle`.
    pub fn finish(&mut self) -> Option<GateState> {
        (self.state != GateState::Idle).then(|| self.enter(GateState::Idle))?
    }

    /// Takes one 16 ms frame (`SUB` samples at 24 kHz) and its voice score.
    /// Appends what STT should hear to `feed`, and returns the new state
    /// if it changed.
    pub fn push(&mut self, score: f32, frame: &[f32], feed: &mut Vec<f32>) -> Option<GateState> {
        self.frames += 1;
        let voice = score >= VOICE;
        self.voice_run = if voice { self.voice_run + 1 } else { 0 };
        self.quiet_run = if score < QUIET { self.quiet_run + 1 } else { 0 };
        match self.state {
            GateState::Idle => None,
            GateState::Armed => {
                self.pre_roll.extend(frame);
                let over = self.pre_roll.len().saturating_sub(PRE_ROLL);
                self.pre_roll.drain(..over);
                if self.voice_run >= OPEN_RUN {
                    feed.extend(self.pre_roll.drain(..));
                    return self.enter(GateState::Open);
                }
                if self.frames >= ARM_TIMEOUT {
                    return self.enter(GateState::Idle);
                }
                None
            }
            GateState::Open => {
                feed.extend_from_slice(frame);
                if self.quiet_run >= CLOSE_RUN {
                    return self.enter(GateState::Closing);
                }
                None
            }
            GateState::Closing => {
                if self.voice_run >= OPEN_RUN {
                    feed.extend_from_slice(frame);
                    return self.enter(GateState::Open);
                }
                feed.extend(std::iter::repeat_n(0.0, frame.len()));
                if self.frames >= CLOSING_MAX {
                    return self.enter(GateState::Idle);
                }
                None
            }
        }
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pushes `n` frames of score `score`, each filled with `value`; returns
    /// the states entered, in order.
    fn run(gate: &mut Gate, n: u32, score: f32, value: f32, feed: &mut Vec<f32>) -> Vec<GateState> {
        let frame = [value; SUB];
        (0..n)
            .filter_map(|_| gate.push(score, &frame, feed))
            .collect()
    }

    #[test]
    fn idle_ignores_everything() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        assert!(run(&mut gate, 100, 1.0, 0.5, &mut feed).is_empty());
        assert!(feed.is_empty());
        assert!(!gate.listening());
    }

    #[test]
    fn eighty_ms_of_voice_opens_with_the_pre_roll() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        assert_eq!(gate.arm(), Some(GateState::Armed));
        assert_eq!(gate.arm(), None);
        // 1 s of quiet fills the 320 ms pre-roll and is kept no longer.
        assert!(run(&mut gate, 62, 0.0, 0.1, &mut feed).is_empty());
        assert!(run(&mut gate, 4, 0.9, 0.2, &mut feed).is_empty());
        assert!(feed.is_empty(), "nothing reaches STT before the gate opens");
        assert_eq!(run(&mut gate, 1, 0.9, 0.2, &mut feed), [GateState::Open]);
        assert_eq!(feed.len(), PRE_ROLL);
        // The pre-roll ends with the 5 voice frames.
        assert!(feed[PRE_ROLL - 5 * SUB..].iter().all(|&x| x == 0.2));
        assert!(feed[..PRE_ROLL - 5 * SUB].iter().all(|&x| x == 0.1));
    }

    #[test]
    fn armed_gives_up_after_six_seconds() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        gate.arm();
        assert!(run(&mut gate, ARM_TIMEOUT - 1, 0.4, 0.0, &mut feed).is_empty());
        assert_eq!(run(&mut gate, 1, 0.4, 0.0, &mut feed), [GateState::Idle]);
    }

    #[test]
    fn eight_hundred_ms_of_quiet_closes_then_silence_until_finished() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        gate.arm();
        run(&mut gate, 5, 0.9, 0.2, &mut feed);
        feed.clear();
        // Scores between QUIET and VOICE are neither: they hold the gate.
        assert!(run(&mut gate, 200, 0.4, 0.3, &mut feed).is_empty());
        assert!(run(&mut gate, CLOSE_RUN - 1, 0.1, 0.3, &mut feed).is_empty());
        assert_eq!(run(&mut gate, 1, 0.1, 0.3, &mut feed), [GateState::Closing]);
        assert!(
            feed.iter().all(|&x| x == 0.3),
            "the open gate passes the audio"
        );
        feed.clear();
        run(&mut gate, 10, 0.1, 0.3, &mut feed);
        assert_eq!(feed.len(), 10 * SUB);
        assert!(feed.iter().all(|&x| x == 0.0), "closing feeds silence");
        assert_eq!(gate.finish(), Some(GateState::Idle));
    }

    #[test]
    fn voice_while_closing_reopens() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        gate.arm();
        run(&mut gate, 5, 0.9, 0.2, &mut feed);
        run(&mut gate, CLOSE_RUN, 0.1, 0.0, &mut feed);
        assert_eq!(gate.state(), GateState::Closing);
        assert_eq!(run(&mut gate, 5, 0.9, 0.2, &mut feed), [GateState::Open]);
    }

    #[test]
    fn closing_ends_by_itself_if_no_utterance_comes() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        gate.arm();
        run(&mut gate, 5, 0.9, 0.2, &mut feed);
        run(&mut gate, CLOSE_RUN, 0.1, 0.0, &mut feed);
        assert_eq!(
            run(&mut gate, CLOSING_MAX, 0.1, 0.0, &mut feed),
            [GateState::Idle]
        );
    }

    #[test]
    fn a_tap_closes_early() {
        let mut gate = Gate::new();
        let mut feed = Vec::new();
        gate.arm();
        assert_eq!(gate.close(), Some(GateState::Idle), "armed, nothing heard");
        gate.arm();
        run(&mut gate, 5, 0.9, 0.2, &mut feed);
        assert_eq!(gate.close(), Some(GateState::Closing));
        assert_eq!(gate.close(), None);
    }
}
