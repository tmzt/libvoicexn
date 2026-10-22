---
title: Tap to talk, gated by earshot voice activity detection
date: 2026-10-22
status: Draft
covers:
  - A cargo feature `vad-earshot`, on top of `stt-moshi-xn`, that adds the
    `earshot` crate (pure Rust, no ONNX)
  - A tap that arms listening; earshot opens the gate when speech starts
    and closes it when speech stops; the STT model steps only while the
    gate is open, and is idle otherwise
  - The utterance repeated by Phonon when the gate closes, as today
  - The screen showing the gate's state
  - Not covered: wake words, barge-in during playback, a VAD without a tap
sources:
  - Owner's request (2026-10-22): "can we pull in the rust voice detection
    library to gate input as a feature, with touch to trigger".
  - `earshot` 1.2.2 on crates.io (MIT OR Apache-2.0, pykeio/earshot; its
    only dependency is `libm`): `Detector::default()`,
    `predict_f32(&[f32])` on frames of exactly 256 samples at 16 kHz
    (16 ms), returning a score in [0, 1], 0.5 suggested as the threshold;
    `reset()`. The network (~40 KiB) is compiled in, with about 8 KiB of
    state per detector.
  - The ONNX-free alternatives ruled out: `voice_activity_detector` and
    `silero-vad-rs` run Silero on ONNX Runtime (`ort`); `webrtc-vad` binds
    WebRTC's C code.
  - voicexn after plan 20261022_02, steps 3 and 4 (`60cf7cb`): the capture
    ring of 24 kHz mono, `SttWorker` stepping every 80 ms frame,
    `Segmenter` ending an utterance after 15 quiet frames, the echo hold,
    and 15 fps while STT is loaded.
related:
  - docs/design/20261022_02_STT_MOSHI_XN_PLAN.md
---

# Tap to talk, gated by earshot voice activity detection

## 1. Goal

With `--features vad-earshot`, voicexn is idle until tapped. After a tap
it listens. The STT model starts stepping when earshot hears speech, and
stops when the speech has ended and the last words are out. Phonon then
repeats what was said, and voicexn is idle again. While idle, the STT
model does no work, so the phone stays cool between turns.

Without the feature, `stt-moshi-xn` behaves as it does now: always
listening.

## 2. Decisions

1. **`vad-earshot = ["stt-moshi-xn", "dep:earshot"]`**, with `earshot =
   "1.2.2"`, default features (std).
2. **The gate runs on the `stt` thread.** That thread already reads the
   24 kHz capture ring. It resamples a copy to 16 kHz with
   `playback::Resampler` and scores each 256-sample frame with earshot.
   That costs well under a millisecond per frame, against 60 ms of STT.
3. **States**, all on the `stt` thread:
   - `Idle`: frames are read and dropped. Neither earshot nor STT runs.
   - `Armed`: after a tap. Earshot scores each frame. Five consecutive
     frames at 0.5 or above (80 ms) open the gate. If nothing opens it
     within 6 s, the state returns to `Idle`.
   - `Open`: each 80 ms frame goes to STT, starting with a pre-roll of the
     last 320 ms heard while `Armed`, so the first word is not clipped.
     Earshot keeps scoring.
   - `Closing`: entered from `Open` once earshot has scored below 0.35 for
     800 ms. STT is fed silence frames for the ASR delay plus the
     `Segmenter`'s 15 quiet frames, so the last words come out and the
     utterance ends. The utterance is then sent, and the state returns to
     `Idle`. Speech scoring 0.5 or above again for 80 ms returns the state
     to `Open`.
4. **The tap.** With the feature: a tap in `Idle` arms. A tap in `Armed`
   or `Open` closes at once (goes to `Closing`). A tap while Phonon speaks
   stops it (`TtsRequest::Stop`) and arms. Without the feature, the tap
   still repeats the last utterance.
5. **The app tells the thread** through a second channel:
   `SttRequest::Arm` and `SttRequest::Close`. The thread reports
   `SttEvent::Gate(GateState)` on every change, for the screen.
6. **Screen:** the second status line shows `tap to talk`, `listening…`,
   `hearing` or `finishing`. The other lines are as now.

## 3. Steps

1. **The gate as a pure state machine.** `src/vad.rs`: `Gate` holds the
   state, the thresholds and timings of decision 3, and the pre-roll.
   `Gate::push(score, frame_24k)` returns what to feed STT, if anything,
   and whether the state changed. Unit tests drive it with scripted
   scores: open after 80 ms of voice, give up after 6 s, close after
   800 ms of quiet, reopen on speech while closing, a pre-roll of 320 ms,
   and a tap that closes early.
   **Done when:** `cargo test --features vad-earshot` passes, and
   `cargo tree -e normal --features vad-earshot` shows `earshot` with
   only `libm` under it, and no `ort`.

2. **The gate on the `stt` thread, and the tap.** Earshot on 16 kHz frames
   resampled from the ring, the requests of decision 5, the tap of
   decision 4, and the status line of decision 6.
   **Done when:** on the S23, the app is idle after launch; a tap, a
   spoken sentence and a pause put it on screen and have it repeated, and
   it is idle again. The log shows no STT steps while idle, and a tap
   followed by 6 s of silence returns it to idle.

## 4. Open questions

1. Are the thresholds (0.5 to open, 0.35 for 800 ms to close) and the 6 s
   timeout right? They are earshot's suggested threshold and common VAD
   timings, still to be tuned on the phone.
