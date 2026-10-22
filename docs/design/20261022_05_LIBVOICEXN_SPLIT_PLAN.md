---
title: libvoicexn, a speech library with voicexn as its demo
date: 2026-10-22
status: Draft
covers:
  - Renaming this repository to `~/src/libvoicexn`, keeping its history,
    plans and `CHANGES.md`
  - A cargo workspace: the root crate `libvoicexn` holds the engine (TTS,
    STT, SentencePiece, vocabulary boost, VAD gate, rings and resampler);
    `examples/voicexn` is the libmorph app as it is today
  - A core that does not depend on libmorph, with libmorph adapters behind
    a `morph` feature
  - `SttConfig` and `TtsConfig` in place of build-time environment
    variables, with a quantization chosen per platform: q4k on Android,
    q8 on the desktop if the A/B in step 4 bears it out
  - Version 0.1.0 of the library, `VERSIONS.md` and the tag `v0.1.0`
  - `examples/voicexn-tty`, a terminal version on `cpal`, built without
    `morph`
  - Not covered: new models, GPU backends, Android outside the libmorph
    example, publishing to crates.io
sources:
  - Owner (2026-10-22): "does it make sense to make ~/src/libvoicexn a
    library and support different quants on mobile and desktop?"; "my
    thinking is this app becomes the demo, maybe even moving into an
    example in the library"; "so a tty version could exist?"; agreed to
    one plan with the tty example as its last step.
  - libmorph's workspace (`~/src/libmorph/Cargo.toml`): `members = [".",
    "crates/libmorph-build", "examples/android/audiocapture"]`, the
    Android example being its own crate with `build.rs`,
    `crate-type = ["lib", "cdylib"]` and `publish = false`.
  - voicexn at `5b2d7e5` (`feat-moshi-xn`): libmorph appears only in
    `lib.rs` (the app), `ui.rs`, `stt::capture` (a `libmorph::Sink` built
    from `CaptureFormat` and `Captured`) and `playback::source` (a
    `libmorph::Source`). The STT quant comes from `VOICEXN_STT_QUANT`
    (`q4k` by default, `q4_0`, `q8_0`), baked in on Android with
    `option_env!`; Phonon loads with `ptts::synth::Quant::Q80`. On the S23,
    q8 steps take 82–133 ms against an 80 ms budget, and q4k 60–72 ms
    (CHANGES.md, plan 20261022_02).
  - `ptts::synth::Quant` at `49b9033`: `F32` (default), `Q80`, `Q81`,
    `Q8k`, `Q6k`, `Q50`, `Q51`.
related:
  - docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md
  - docs/design/20261022_02_STT_MOSHI_XN_PLAN.md
  - docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md
  - docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md
---

# libvoicexn, a speech library with voicexn as its demo

## 1. Goal

Speech in and out on xn, without ONNX, as a library any app can use.
voicexn, the libmorph app built under plans 01 to 04, becomes the
library's demo in `examples/voicexn` and keeps working on the S23 and the
desktop as it does now. A second example, `examples/voicexn-tty`, runs the
same engine in a terminal with no window and no libmorph. The STT
quantization is chosen per platform instead of once at build time.

## 2. Decisions

1. **One repository, renamed.** `~/src/voicexn` becomes `~/src/libvoicexn`
   (`mv`; there is no remote). History, the plans, `CHANGES.md` and the
   `models/` links into `~/src/common-models` stay. Plans 01 to 04 keep
   their text. Where they name the `voicexn` crate, it now means
   `examples/voicexn` for the app and `libvoicexn` for the engine; this
   is recorded in `CHANGES.md`, not edited into them.

2. **Workspace layout:**

   ```
   Cargo.toml              package libvoicexn + [workspace]
                           members = [".", "examples/voicexn", "examples/voicexn-tty"]
   src/lib.rs              pub mod tts, stt, spm, boost, vad, playback, capture, config;
                           #[cfg(feature = "morph")] pub mod morph;
   examples/voicexn/       package voicexn, publish = false
     Cargo.toml            lib "voicexn" (cdylib + rlib), bin "voicexn"
     build.rs              libmorph-build, app id dev.tmzt.voicexn, RECORD_AUDIO
     src/lib.rs            App, android_main          (today's src/lib.rs)
     src/ui.rs             Screen                     (today's src/ui.rs)
     src/main.rs           desktop entry, --say, --transcribe
   examples/voicexn-tty/   package voicexn-tty, publish = false (step 5)
   scripts/, vocab.txt, models/, .cargo/config.toml   stay at the root
   ```

   `scripts/android.sh` builds `-p voicexn` and finds the Gradle project
   under `target/` as now. The desktop examples are run from the root, so
   `models/<name>` and `vocab.txt` resolve as today.

3. **Features belong to the library:** `stt-moshi-xn`, `vad-earshot`,
   `stt-moshi-xn-boost` as now, plus `morph = ["dep:libmorph"]`. Each
   example declares the same names and forwards them
   (`stt-moshi-xn = ["libvoicexn/stt-moshi-xn"]`), so `build.rs`'s
   `CARGO_FEATURE_STT_MOSHI_XN` check and `scripts/android.sh -- --features
   "vad-earshot stt-moshi-xn-boost"` work unchanged.

4. **No libmorph in the core.** The audio ends are plain:
   - `capture::Mic`: what today's capture closure does (downmix, peak,
     echo hold, resample to 24 kHz, push to the ring, count drops), as a
     struct with `Mic::new(in_rate: u32, channels: u16, speaking:
     Arc<AtomicBool>, seconds: u32) -> (Mic, stt::Capture)` and
     `Mic::push(&mut self, interleaved: &[f32])`. It allocates nothing on
     `push` for blocks up to a second, as now.
   - `playback::Consumer::fill(&mut self, out: &mut [f32])`, as now.
   - `morph::sink(mic: Mic) -> libmorph::Sink` and
     `morph::source(consumer: Consumer) -> libmorph::Source`, each a
     one-line closure. `stt::capture(format, speaking, seconds)` moves here
     as `morph::capture`, built on `Mic`.

   The library sets no environment variables and starts no rayon pool:
   `RAYON_NUM_THREADS` stays the app's job (examples/voicexn keeps
   today's 4/2 on Android).

5. **Configuration replaces build-time variables:**

   ```rust
   pub enum SttQuant { Q4k, Q4_0, Q8_0 }        // file model.{q4k,q4_0,q8}.gguf
   pub struct SttConfig {
       pub model_dir: PathBuf,
       pub quant: SttQuant,
       pub vocab: Option<PathBuf>,              // boost feature only
       pub boost: (f32, f32),                   // start, cont; boost feature only
   }
   pub struct TtsConfig {
       pub model_dir: PathBuf,
       pub quant: ptts::synth::Quant,           // Q80 on both, as now
       pub record: Option<PathBuf>,
   }
   ```

   - `SttConfig::for_platform(model_dir)` picks `Q4k` on Android and the
     step 4 winner on the desktop (`Q8_0` unless the A/B says otherwise),
     then applies `VOICEXN_STT_QUANT`, `VOICEXN_VOCAB` and `VOICEXN_BOOST`
     if set. `TtsConfig::for_platform` does the same for Phonon.
   - `SttWorker::spawn(config, capture)`, `TtsWorker::spawn(config,
     playback)`, `Stt::load(&config)` and `transcribe_file(&config, wav)`
     take the config instead of a path. The examples add `--quant
     q4k|q4_0|q8` on the desktop; on Android `VOICEXN_STT_QUANT` at build
     time still overrides, through the example's `android_main`.
   - `scripts/fetch-stt.sh` builds `q4k q8` by default (`STT_QUANTS`
     unchanged as the override); `scripts/push-stt.sh` pushes only q4k.

6. **The public API** is what the examples use: the modules of decision 2,
   the worker types and their events and requests, `Mic`, `Producer`,
   `Consumer`, `ring`, `Resampler`, `Gate`, `Decoder`, `VocabBooster`, the
   configs and the `morph` adapters. Anything the examples do not need
   stays `pub(crate)`.

7. **Versioning from here on.** `libvoicexn` is 0.1.0, tagged `v0.1.0` on
   the commit that records it in `VERSIONS.md` (AGENTS.md Rule 5). The
   examples are `publish = false` at 0.1.0 and are not versioned under
   Rule 5. `AGENTS.md`'s opening paragraph changes from "an application"
   to "a library, with voicexn as its demo"; the rules themselves do not
   change.

8. **The tty example** is a plain binary with `libvoicexn` (no `morph`)
   and `cpal` for the default input and output devices. The input
   callback calls `Mic::push`; the output callback calls
   `Consumer::fill`. Partials are redrawn on one line with `\r`, each
   utterance prints as `> text` and Phonon repeats it. Enter on stdin
   does what a tap does (decision 4 of plan 03 with `vad-earshot`;
   otherwise it repeats the last utterance). Flags: `--quant`, `--voice`,
   `--vocab`. `cpal` is a binding to the system audio APIs, not an
   inference runtime, and only this example depends on it.

## 3. Steps

0. **Before starting.** `feat-moshi-xn` is merged into `main` (plan 04
   step 3 done or left open by the owner), and the directory is renamed
   to `~/src/libvoicexn`, with the session reopened there.
   **Done when:** `git -C ~/src/libvoicexn status` is clean on `main`, and
   `cargo build --features "vad-earshot stt-moshi-xn-boost"` passes there.

1. **The workspace and the move.** Three commits, each of one crate
   (Rule 3.1), each building:
   1. Root: package renamed `libvoicexn` (lib name `libvoicexn`), with the
      app still inside.
   2. `examples/voicexn`, the crate's first commit, which also adds it to
      `[workspace] members`: the app files of decision 2, using the
      engine from `libvoicexn`.
   3. Root: the app files, `build.rs`, the bin, `crate-type = ["cdylib"]`
      and the libmsdf, libcontrols and logger dependencies removed.

   `scripts/android.sh` moves to `-p voicexn` in its own commit.
   **Done when:** every crate passes fmt, clippy and tests; on the desktop
   `--say` writes the same WAV as before (sample-equal) and
   `--transcribe` gives the same text; and the APK from
   `scripts/android.sh` runs a tap-to-talk turn on the S23 with the boost.

2. **No libmorph in the core.** `capture::Mic`, the `morph` module and
   feature (decision 4); the example turns on `morph`.
   **Done when:** `cargo tree -p libvoicexn -e normal --features
   "vad-earshot stt-moshi-xn-boost"` shows no `libmorph`, `wgpu` or
   `libmsdf`; the capture tests run against `Mic` without libmorph; and
   the S23 turn of step 1 still works.

3. **The configs.** Decision 5, with the examples using
   `for_platform` and `--quant`.
   **Done when:** no `VOICEXN_*` variable is read outside `config.rs` and
   the examples' entry points; tests cover `for_platform` with and
   without the overrides; the S23 loads q4k without any build-time
   variable.

4. **The desktop quant.** On the Mac: the plan 04 A/B sentences and a
   one-minute recording through `--transcribe` with q4k and q8, at the
   default thread count. Mean and worst step time and both transcripts
   go to `out/stt/` with a log entry.
   **Done when:** the log names the desktop default, and
   `SttConfig::for_platform` uses it. q8 wins if it keeps the mean step
   under 40 ms (half the frame) and its transcripts are at least as
   correct; otherwise q4k stays.

5. **0.1.0.** `VERSIONS.md` with the 0.1.0 entry listing the public API
   of decision 6, the version in `Cargo.toml`, and the tag. `AGENTS.md`
   and the README's introduction updated (decision 7), each in its own
   commit.
   **Done when:** `git tag` shows `v0.1.0` on the `VERSIONS.md` commit,
   and `cargo doc -p libvoicexn --no-deps --all-features` builds with no
   warnings.

6. **The tty example.** `examples/voicexn-tty` of decision 8, in its
   first commit with its workspace entry.
   **Done when:** it builds with `libvoicexn` without `morph` (`cargo
   tree -p voicexn-tty` shows no `libmorph` or `wgpu`), and on the Mac a
   spoken sentence is shown and repeated by Phonon, with `--features
   vad-earshot` an Enter-armed turn returns to idle, and `dropped` stays
   0 for a minute of speech. The run's log goes to `out/tty/`.

## 4. Open questions

1. Does libmorph's desktop build capture the microphone today? The tty
   example does not depend on the answer, but the desktop `voicexn`
   example's live STT does.
2. Should `examples/voicexn` keep the name, or become `voicexn-android`
   now that there are two? This plan keeps `voicexn`, so the package,
   app id and scripts do not change.
