---
title: A speech session in libvoicexn, with no libmorph and cpal as an option
date: 2026-10-29
status: In implementation
covers:
  - A `session` module that runs both workers for an app: `Session`,
    `SessionConfig`, `Action`, `SessionEvent` and the `Pump` that tracks
    speaking and listening, moved from flankr's `flankr-audio`
  - `playback::Playback` in the core: a flush at any time, the rest of the
    buffer zeroed, and an optional cue sound
  - A `cpal` feature that opens the default devices, for apps without
    libmorph
  - Removing the `morph` feature and the libmorph dependency, replaced by
    two closures documented in the README
  - The push scripts taking the app id, the models folder and the staging
    folder, with the fixes flankr's copies made
  - libvoicexn 0.4.0, the next minor (breaking) release after 0.3.0
  - Not covered: flankr moving onto 0.4.0 (flankr's own work, in its own
    commits), `android.sh` (libmorph-build plumbing, a candidate for
    libmorph), word timings or answer audio for RoundTwo
sources:
  - "The owner, 2026-10-29 (in RoundTwo's session): break flankr-audio's generic speech plumbing out into libvoicexn with the push and quantisation scripts; the two-line libmorph closures are the documented way for libmorph apps, so an app does not pull in cpal just to adapt audio; document it clearly in the README and CHANGES; make it the next breaking, minor change."
  - A read-only review of `~/src/flankr/crates/flankr-audio` against this repository at `ce015d7` (2026-10-29), run from RoundTwo's session
  - ~/src/flankr/crates/flankr-audio/src/{lib,pump,playback,devices,morph}.rs at flankr-audio 0.2.0
  - ~/src/flankr/apps/companion/scripts/push-{model,stt,vocab}.sh
related:
  - docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md (decisions 4 and 8, which this plan changes; that plan is frozen and is not edited)
  - docs/design/20261022_07_STT_RUNTIME_VOCAB_PLAN.md (0.3.0, which this release follows)
  - flankr's companion app plan (~/src/flankr/docs/design/20261022_04_COMPANION_APP_PLAN.md)
  - RoundTwo's interview coach plan (~/src/RoundTwo/docs/design/20261029_01_INTERVIEW_COACH_PLAN.md), the second app on the session
---

# A speech session in libvoicexn

## Why

flankr's companion app built `flankr-audio` on libvoicexn 0.2.0. About
three quarters of it is generic: a session that wires the microphone, the
playback ring and both workers; a pump that says when the app is speaking
and listening; playback that can be flushed at any moment and plays a cue;
and opening the default devices with cpal. RoundTwo needs the same. It
belongs here, so neither app copies it.

libvoicexn's `morph` feature pins libmorph at `bb0174d` (0.3.0-era), and
libmorph sets `links = "libmorph"`, so a build holds only one libmorph
revision. flankr already turns the feature off and copies its two
closures. Any pin here would force every app onto libvoicexn's libmorph,
and a libvoicexn release on every libmorph move.

## Decisions

1. **The core opens no device and names no app framework.**
   `Session::spawn` returns the two audio ends, `Mic` and `Playback`; the
   app decides who calls `Mic::push` and `Playback::fill`.
2. **libmorph apps use two closures**, documented in the README section
   "Using libvoicexn in a libmorph app": a capture `Sink` calling
   `mic.push(captured.samples)` and an output `Source` calling
   `playback.fill(out)`. This is the intended path for libmorph apps, not
   a workaround: on Android libmorph must own the streams (the
   `VoiceRecognition` capture mode, the permission, suspend and resume).
   The `morph` feature, `src/morph.rs` and the libmorph dependency are
   removed. This changes decision 4 of the split plan.
3. **`cpal` is an optional feature** for apps without libmorph (tty
   tools, command lines, tests): `Session::open_default` and
   `Session::open_default_output`. cpal is the version libmorph's
   libmobile uses (0.18.2), so a build with both holds one. This reverses
   the split plan's decision 8 ("only this example depends on it"), and
   `voicexn-tty` moves onto the feature.
4. **`Playback` wraps `Consumer`** and is what apps hand to the output.
   `Consumer` is unchanged, so its own flush on a `Stop` that reaches the
   worker mid-synthesis still works; `Playback` adds a flush that works at
   any time, by a counter.
5. **No flankr types.** flankr's `Event` and `Input` stay in flankr; what
   `flankr-audio` wrapped in `AudioEvent::Input` becomes top-level
   `SessionEvent` variants. The vocabulary file has no default path.
6. **Version 0.4.0.** Removing `morph` and adding `session` is not
   strictly additive (Rule 5.4). It follows 0.3.0 (plan 07, step 5). If
   0.3.0 is not tagged when step 1 starts, this plan waits for it.

## Interfaces

### `playback` (core)

```rust
pub struct Playback { /* Consumer, scratch, flush seen, cue */ }
impl Playback {
    /// `consumer` from `playback::ring`; `flush` is moved by the session
    /// on every Stop, and the ring is drained when it moves.
    pub fn new(consumer: Consumer, flush: Arc<AtomicU32>) -> Self;
    /// Mix `cue` (interleaved, at the output's rate and channels) in once
    /// each time `cues` moves.
    pub fn with_cue(self, cue: Vec<f32>, cues: Arc<AtomicU32>) -> Self;
    /// Fill `out`; what the ring does not cover is zeroed.
    pub fn fill(&mut self, out: &mut [f32]);
}
/// A short two-tone chime at `rate`, mono; the session's default cue.
pub fn chime(rate: u32) -> Vec<f32>;
```

`fill` neither blocks nor allocates, as `Consumer::fill`.

### `session` (`stt-moshi-xn` and `vad-earshot`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format { pub rate: u32, pub channels: u16 }

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub stt: SttConfig,
    pub tts: TtsConfig,
    /// The Phonon voice for every Speak; None is the checkpoint's own.
    pub voice: Option<String>,
    /// If it exists at spawn, the STT boost reads it in place of the list
    /// SttConfig::for_platform found (VOICEXN_VOCAB included); written on
    /// Action::Vocabulary. None writes and reads nothing.
    pub vocab_file: Option<PathBuf>,
}
impl SessionConfig {
    /// models/phonon-31m-en and models/stt-1b-en_fr with for_platform.
    pub fn new(models: impl AsRef<Path>) -> Self;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action { Speak(String), Stop, Arm, Vocabulary(Vec<String>), Cue }

#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    Utterance(String), Partial(String),
    Listening(bool), Speaking(bool),
    Gate(GateState),
    SttLoaded { load_ms: u32 }, SttLoadFailed(String),
    TtsLoaded { load_ms: u32, voices: usize, names: Vec<String> },
    TtsLoadFailed(String),
    Speed { step_ms: f32, dropped: u64 },
    Spoken { audio_ms: u32, synth_ms: u32 }, SpeakFailed(String),
    Vocabulary { phrases: usize },                 // the running model's list, from SttEvent::Vocabulary
    VocabularySaved { path: PathBuf, phrases: usize }, VocabularyFailed(String),
}

pub struct Session { /* workers, Pump, flush and cue counters, voice */ }
impl Session {
    pub fn spawn(cfg: SessionConfig, input: Format, output: Format) -> (Session, Mic, Playback);
    pub fn act(&mut self, action: Action);
    pub fn poll(&mut self) -> Vec<SessionEvent>;
    pub fn set_voice(&mut self, voice: Option<String>);
    pub fn voice(&self) -> Option<&str>;
}

pub mod pump {
    pub struct Pump;  // new, on_action, on_stt, on_tts, tick(playing)
    pub fn tts_request(action: &Action, voice: Option<&str>) -> Option<TtsRequest>;
    pub fn stt_request(action: &Action) -> Option<SttRequest>;
    pub fn gate_listening(state: GateState) -> bool;
    pub fn vocab_text(words: &[String]) -> (String, usize);
    pub fn write_vocab(path: &Path, words: &[String]) -> std::io::Result<usize>;
}
```

`Speaking(true)` is sent on the first Speak and `Speaking(false)` only
when nothing is queued, nothing is being made and the ring is dry, as
flankr's pump. `Action::Vocabulary` sends `SttRequest::Vocabulary` to the
running model (0.3.0) and, with `vocab_file`, also writes the file for
the next start. `Action::Cue` is flankr's `Chime`, renamed, since the cue
is any sound. Under `stt-moshi-xn-boost` only, `Vocabulary` changes the
running model; without it the file is still written. The header line
`vocab_text` writes names libvoicexn, not flankr-audio.

### `cpal` feature

```rust
impl Session {
    /// The default input and output devices, opened with cpal; the
    /// streams are held by the session.
    pub fn open_default(cfg: SessionConfig) -> Result<Session, String>;
    /// Only the output; the app pushes into the returned Mic itself.
    pub fn open_default_output(cfg: SessionConfig, input: Format) -> Result<(Session, Mic), String>;
}
```

### Scripts

`scripts/push-model.sh`, `push-stt.sh` and `push-vocab.sh` read:

| Variable | Default | Meaning |
| --- | --- | --- |
| `APP_ID` | `dev.tmzt.voicexn` | the debuggable app to `run-as` |
| `VOICEXN_MODELS` | `<repo>/models` | the folder holding `phonon-31m-en` and `stt-1b-en_fr` |
| `STAGE` | `/data/local/tmp/voicexn-<model>` | where files are staged on the phone, kept between runs |

They gain flankr's fixes (`stat -L` for sizes, `"$SRC/."` so a linked
folder pushes its contents) and keep this repository's (deleting a stale
`mimi.safetensors`). An app runs them from a libvoicexn checkout, for
example `APP_ID=com.tmzt.roundtwo ~/src/libvoicexn/scripts/push-stt.sh`.
`fetch-model.sh` and `fetch-stt.sh` (which quantises with xn-moshi's
`quantize`) are unchanged and documented beside them.

## Steps

0. **Before starting.** Plan 07 is `Implemented` and `v0.3.0` is tagged.
   **Done when:** `git tag` lists `v0.3.0`.
1. **`Playback` in the core.** Ported from flankr-audio's `playback.rs`
   with its tests, as decision 4.
   **Done when:** `cargo test -p libvoicexn` covers a flush by counter
   draining the ring, the tail zeroed, and a cue mixed once per move.
2. **The session.** `session` and `session::pump`, with flankr-audio's
   pump and session tests, the vocabulary through
   `SttRequest::Vocabulary`.
   **Done when:** `cargo test -p libvoicexn --features "stt-moshi-xn
   vad-earshot stt-moshi-xn-boost"` passes, and on the Mac a Stop during
   a long Speak goes silent no slower than flankr-audio does, measured
   both ways and logged under `out/session/<YYYYMMDD_NN_stop>/`.
3. **`cpal`.** The feature, and `voicexn-tty` on `Session::open_default`.
   **Done when:** `voicexn-tty` hears an utterance and repeats it on the
   Mac, and `cargo tree -p libvoicexn` without `cpal` holds no cpal.
4. **No libmorph.** `morph` and `src/morph.rs` removed; `examples/voicexn`
   uses `Session::spawn` and the README's two closures, on its own
   libmorph pin.
   **Done when:** `cargo tree -p libvoicexn --all-features` holds no
   libmorph, and `examples/voicexn` listens and speaks on the Mac and on
   the S23.
5. **Scripts.** As "Scripts" above.
   **Done when:** with `APP_ID` unset the scripts push to the voicexn
   example as before, and with `APP_ID` set they push to another
   debuggable app.
6. **0.4.0.** The version, the `VERSIONS.md` entry (every change above,
   the `morph` removal marked **Breaking**) and the tag `v0.4.0`, pushed
   with the branch.
   **Done when:** `git ls-remote origin` shows `v0.4.0`.

After this, flankr moves `flankr-audio` onto 0.4.0 in flankr's own commits
(a thin layer: `action(&Event)`, `SessionEvent` to `Input`, its vocabulary
path, its own closures; a minor bump of flankr-audio), and RoundTwo's
step 5 starts on it.

## Amendment — 2026-10-29 10:54 EDT: approved; implementation started

The owner (2026-10-29): "approve the libvoicexn path, I mean push the next
minor/breaking libvoicexn version to github". `status` is now
`In implementation`, and the document is immutable from here on
(AGENTS.md Rule 1.5). Step 0 holds: 0.3.0 is committed as `71a7883` and
tagged `v0.3.0`, to be pushed with this release. Step 1 is in the commit
that carries this amendment.
