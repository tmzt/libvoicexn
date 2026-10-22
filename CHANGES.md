# CHANGES

Addendum log (AGENTS.md Rule 0). Entries are appended, never edited.

## 2026-10-22 08:45 EDT — Phonon 31m text-to-speech on Android plan

New planned work: the voicexn application crate, a libmorph app that
speaks preset phrases with Phonon 31m through the `ptts` crate on xn (CPU,
q8, no ONNX Runtime), streamed into libmorph's audio output, with a
libmsdf/libcontrols screen. Desktop first, then the Galaxy S23.
Speech-to-text is left for a later plan.
Plan: `docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md` (Draft),
built in the five steps of section 5.

## 2026-10-22 08:50 EDT — Phonon TTS plan: voices, Apple silicon, implementation starts

The owner chose the web demo's short voice list (37 voices plus
`default`) and noted desktop builds run on Apple silicon; both are recorded
as one amendment to `docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md`.
The owner asked to start: step 1 sets the plan to In implementation.
Dependencies: libmsdf is taken at `b48d066` (0.4.0), the revision libcontrols
0.5.0 pins, so the graph holds one libmsdf.

## 2026-10-22 08:54 EDT — Phonon TTS plan, step 1: small departures

Plan `docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md`, step 1:
- `App::limits` asks for at least 8 storage buffers per shader stage.
  libmsdf's fragment stage binds five, and libmorph's downlevel default of
  four fails device bring-up.
- `scripts/android.sh` is added to the layout of section 3. It is the
  two-stage build (cargo, then gradle) of libmorph's
  `examples/android/build.sh`, for `dev.tmzt.voicexn`.

## 2026-10-22 08:59 EDT — Phonon TTS plan, step 2: small departures

Plan `docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md`, step 2:
- `ptts` comes from git at tag v0.4.1 (`49b9033`), not crates.io: 0.4.1
  is not published there yet (0.4.0 is the latest). Decision 1 named
  `version = "0.4.1"`. Switch back once it is published.
- `TtsWorker::spawn(model_dir, record: Option<PathBuf>)`: `record` writes
  each utterance as a WAV file, which `--say` uses. Step 3 adds the ring
  `Producer` that section 3 names. `TtsWorker::wait` blocks for the next
  event, for `--say`. `TtsEvent::Failed` reports a failed utterance.
- `scripts/fetch-model.sh` uses `curl` against both pinned revisions rather
  than `hf download`, so it needs no Python tooling.

## 2026-10-22 09:07 EDT — Phonon TTS plan, step 3: small departures

Plan `docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md`, step 3:
- The step was proven on the Galaxy S23 Ultra, not only the desktop. The
  headless `--morph-script` run opens no audio output, and the owner tests
  on the phone. That brought forward two parts of step 5:
  `scripts/push-model.sh`, and `RAYON_NUM_THREADS=2` in `android_main`.
  The thread-count comparison is still step 5's.
- The ring is `rtrb` 0.3. On `Stop`, the producer raises a flush flag that
  the audio thread clears after dropping the queued audio, instead of
  section 3's generation counter. The producer waits at most 100 ms for it,
  since a paused output never answers.
- The resampler holds each chunk's last sample for the next chunk, so a
  chunk that ends exactly on an output position leaves no seam. The final
  sample of an utterance (42 µs) is not played.
- `synth_ms` leaves out the time spent waiting for room in the ring, so the
  RTF of utterances longer than the ring's 2 s stays a measure of synthesis.

## 2026-10-22 09:13 EDT — Speech-to-text on xn-moshi plan

New planned work, on branch `feat-moshi-xn`: a `stt-moshi-xn` feature that
transcribes the microphone live with Kyutai's `stt-1b-en_fr` (q8) through
`xn-moshi` 0.2.6, on a `stt` thread fed by libmorph capture, half-duplex
with the Phonon TTS, and a tap that speaks the transcript back. Step 2
measures the phone's step time against the 80 ms frame before screen work.
Plan: `docs/design/20261022_02_STT_MOSHI_XN_PLAN.md` (Draft), built in the
four steps of section 4.

## 2026-10-22 09:17 EDT — Models live in ~/src/common-models

Owner's instruction: downloaded models go in `~/src/common-models`, and
the project links to them. `scripts/fetch-model.sh` (plan 20261022_01,
step 2) now writes `${COMMON_MODELS:-~/src/common-models}/phonon-31m-en`
and links `models/phonon-31m-en` to it. It takes no directory argument any
more. `scripts/fetch-stt.sh` (plan 20261022_02, step 2) does the same for
`stt-1b-en_fr`. Section 3 of both plans names `models/<name>`, which is
now a symlink. The models already downloaded were moved, not fetched again.

## 2026-10-22 09:17 EDT — STT plan, step 1: small departures

Plan `docs/design/20261022_02_STT_MOSHI_XN_PLAN.md`, step 1:
- The capture API is `stt::capture(format, speaking, seconds) -> (Sink,
  Capture)` with `CaptureStats` (peak, dropped), not section 3's
  `capture_sink(.., ring)`. The sink owns its ring and hands back the
  reading end.
- `playback::Consumer::speaking()` is the shared flag of decision 6. It is
  set after every fill that played something.
- Until step 3, `App::update` empties the capture ring each frame.

## 2026-10-22 09:36 EDT — STT plan, step 2: the phone misses the 80 ms budget; stopped

Plan `docs/design/20261022_02_STT_MOSHI_XN_PLAN.md`, step 2. The step's own
gate stops the plan here, and the result goes to the owner.
- The transcript is right on both machines. Phonon's demo sentence comes
  back as "Phone on turns, text into natural speech, write on your device
  with no cloud round trip."
- M4: load 2.6–3.4 s. Mean step 31.5 / 26.5 / 22.4 / 26.0 ms at 1 / 2 / 4 / 8
  threads, and 67 ms with rayon's default of every core.
- S23 Ultra, five runs started 10 s after Phonon loaded: load 7.3–10.3 s.
  Mean step 93.6 ms at 2 threads, 132.9 and 82.3 ms at 3, 104.0 and 83.5 ms
  at 4. p95 107–396 ms. None is under the 80 ms frame.

Departures in the step-2 code:
- `model.q8.gguf` holds the LM's q8_0 linears and Mimi's *encoder*. xn-moshi's
  quantizer leaves out everything under `mimi.decoder*` and `mimi.upsample`,
  and `Mimi::load` builds the decoder too. So Mimi loads from
  `mimi.safetensors` and the LM from the GGUF (`VB::load_gguf`,
  `.pp("lm")`, `Q80F32`). The phone holds both, 1.6 GB.
- xn-moshi 0.2.6 on crates.io has no `Config::asr_delay_in_tokens`. The delay
  is `stt_config.audio_delay_seconds` × 12.5 (6 frames).
- `xn` and `serde_json` are direct optional dependencies of the feature.
- The phone's debug hook is a `transcribe.wav` in the data directory,
  transcribed 10 s after launch, rather than app arguments. A build-time
  `VOICEXN_THREADS` sets `RAYON_NUM_THREADS` (default 2) for the comparison.
- `scripts/push-stt.sh` stages each file, skips files already there at full
  size, and retries, because wireless adb dropped the 1.4 GB push twice.

## 2026-10-22 10:01 EDT — STT plan, step 2: q4k keeps up on the S23

Plan `docs/design/20261022_02_STT_MOSHI_XN_PLAN.md`, step 2, after the stop
recorded above. The owner chose 4-bit weights. This departs from decision 3
(q8_0): the LM is quantized to q4k. Nothing else changes: the same xn-moshi
quantizer, CPU, and Mimi from `mimi.safetensors`.
- The q4_0 and q4k GGUFs are 974 MB, against 1.37 GB for q8. The quantizer
  quantizes only the transformer linears; the embeddings and Mimi's encoder
  keep their size.
- M4, 4 threads: mean step q8_0 21.4 ms, q4_0 19.0 ms, q4k 17.8 ms. All
  three give the same transcript, except that q4k writes "phone"
  lowercase.
- S23 Ultra, q4k: mean step 88.7 ms (2 threads) and 69.6 ms (3 threads).
  Three runs at 4 threads gave 60.6, 72.3 and 66.4 ms, p95 66.8–83.2 ms,
  with the transcript unchanged. q4_0 at 4 threads: 79.2 ms. q4k at 4
  threads keeps the mean under the 80 ms frame, which is the gate. Single
  frames over 80 ms are absorbed by the 4 s capture ring. The margin is
  10–25%, measured with the phone warm from back-to-back runs.
- q4k takes 13–23 s to load on the phone, against 7–10 s for q8.
- Defaults: `VOICEXN_STT_QUANT=q4k`, and `RAYON_NUM_THREADS=4` when
  `stt-moshi-xn` is on (2 without it). `fetch-stt.sh` and `push-stt.sh`
  default to q4k. `STT_QUANTS` names others.
- Open for step 3: `RAYON_NUM_THREADS` is one pool shared with Phonon. In
  half-duplex both run at once while voicexn speaks.

## 2026-10-22 10:15 EDT — STT plan, steps 3 and 4: live, and repeated by Phonon

Plan `docs/design/20261022_02_STT_MOSHI_XN_PLAN.md`, steps 3 and 4. The
owner asked for "transcribe then repeat with phonon, display resulting
text". That answers open question 2: an utterance is spoken back
automatically when it ends, and a tap repeats the last one. Departures:
- An utterance ends when 15 frames (1.2 s) pass with no new word after at
  least one (`stt::Segmenter`).
- Decision 6 changes: while voicexn speaks, and for 0.3 s after
  (`ECHO_HOLD_S`), the capture sink pushes *nothing* rather than zeros. The
  model pauses instead of stepping through silence while Phonon has the
  CPU. Its clock does not need to run on.
- Dropped samples are counted from the end of the model load: the 4 s ring
  overflows while the 16–24 s load runs, and that is not a live drop.
- The screen is a persistent libcontrols `Router` (`ui::Screen`): one
  `Label` node per line, edited only when the text, colour or layout
  changes. The draw list is rebuilt only then; other frames draw the list
  already built.
- At the owner's request, the frame rate is capped at 15 fps while STT
  listens, by waiting in `update`. libmorph renders as fast as presentation
  allows (120 Hz on the S23). It is plain wgpu, so the app paces itself.
- The step-2 `transcribe.wav` debug hook is gone from the app.
  `--transcribe` stays on the desktop.
- Thermal: after about an hour of back-to-back runs the S23 reached thermal
  status 3 (SEVERE, AP 68 °C, skin 45 °C), and the live step rose to
  81–141 ms. Cooled to status 1 with the 15 fps cap, it steps at 60–66 ms.

## 2026-10-22 10:18 EDT — Tap to talk with earshot VAD plan

New planned work on `feat-moshi-xn`: a `vad-earshot` feature (on top of
`stt-moshi-xn`) that uses the pure-Rust `earshot` 1.2.2 VAD. A tap arms,
earshot opens and closes the gate, and the STT model is idle between turns.
Plan: `docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md` (Draft), two steps.

## 2026-10-22 10:27 EDT — Vocabulary boost plan

New planned work on `feat-moshi-xn`: a `stt-moshi-xn-boost` feature that
biases Kyutai STT toward a phrase list through xn-moshi's `TokenBooster`.
It needs SentencePiece encoding in `src/spm.rs`. The first list is the
`lib*` folders in `~/src`.
Plan: `docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md` (Draft), three steps.

## 2026-10-22 10:33 EDT — Vocabulary boost plan, step 2: the A/B, and departures

Plan `docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md`, step 2. A/B on the
M4: Phonon WAVs of three sentences that use all nine names, through
`--transcribe` without and with the boost.
- Plain: "lib controls", "Libem's DF", Libmorph, Libmobile, Libdevice,
  Libshader, "Libdeni", "Libdpmdin", "Libestol".
- Boost 1.5/4 (the plan's default): only "Libm's DF" changed.
- Boost 1.5/8: Libgenai right, "Libms DF". Nothing else changed, and the
  plan-02 demo sentence came out the same.
- Boost 1.5/12: Libmsdf right as well, but sentence 3 stops after
  "Libgenai,". Boost 2/16: libcontrols right too, but sentence 3 stops after
  "Libshader,". Too much bias leaves the model inside a word that never
  ends, so it is never flushed.
- Phonon's renderings of the made-up names ("Libdpmdin") limit what this
  A/B can show. Step 3, with a real voice, is the measure.

Departures:
- Decision 4: a pad (3) or silence pad (4) does not end a word. The model
  writes pads between the pieces of one word, and resetting on them
  dropped every match mid-word. A word starts at a token whose piece begins
  with `▁` (`Decoder::starts_word`). Only end of paragraph (0) and end of
  stream (2) clear the matches. First tokens get `start` at every step,
  not only after a word end.
- The default `cont` is 8.0, not 4.0 (above).
- Decision 3: the encoder is `Decoder::encode` on the existing type, not a
  new `spm::Model`.
- `find_vocab` also takes `VOICEXN_VOCAB`.

## 2026-10-22 13:31 EDT — libvoicexn split, step 1: the workspace and the move

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 1, in
`c47bc3e`, `7a25534`, `ceb8f54` and `a3528ec`.
- Decision 1: where plans 01 to 04 name the `voicexn` crate, the app
  (`lib.rs`'s `App` and `android_main`, `ui.rs`, `main.rs`,
  `build.rs`) is now `examples/voicexn`, and the engine (`tts`,
  `stt`, `spm`, `boost`, `vad`, `playback`) is `libvoicexn` at the
  root. Log targets follow: the engine logs as `libvoicexn::stt`, the app
  as `voicexn`.

Departures:
- Step 1.1: `build.rs`'s `lib_name` followed the lib to `libvoicexn`,
  and libmorph-build names the Gradle project after the package, so
  between 1.1 and 1.2 the project was `gradle/libvoicexn` and
  `scripts/android.sh` did not find it. 1.2 and the `-p voicexn` commit
  put it back at `gradle/voicexn`. Between 1.2 and 1.3 the root and the
  example each had a bin named `voicexn`.
- Step 1's **Done when:** "sample-equal" does not hold as written: Phonon's
  `SpeechOptions::default()` has no seed, and two runs of the old build
  differ (72 bytes of one 3.4 s WAV). The check was made with an
  uncommitted `.seed(0)` in both trees and `RAYON_NUM_THREADS=1`: the
  example's WAV equals the old build's byte for byte, and the old build
  still differed from itself by 41 bytes in one of two runs. The
  `--transcribe` text of `out/boost/s1–s3.wav` is identical.

## 2026-10-22 13:36 EDT — libvoicexn split, step 2: departures

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 2, in
`3b4f993` and `62816e1`.
- `capture` is behind `stt-moshi-xn`, as `stt` is: `Mic::new` returns
  `stt::Capture`. `CaptureStats`, `Capture`, `STT_RATE` and
  `ECHO_HOLD_S` stay in `stt`; `morph::sink` and `morph::capture` are
  behind `stt-moshi-xn` too.
- The library commit removes `stt::capture` and `playback::source`, so
  `examples/voicexn` does not build at `3b4f993`, only at `62816e1`.

## 2026-10-22 13:40 EDT — libvoicexn split, step 3: departures

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 3, in
`0a82a48`, `8c805d9` and `7fc023c`.
- `SttConfig::vocab` and `boost` exist only with `stt-moshi-xn-boost`
  (decision 5's "boost feature only"). `SttQuant` adds `parse`, `file`,
  `for_platform` and the constants `ANDROID` and `DESKTOP`; `parse`
  takes `q8` as well as `q8_0`. A bad `VOICEXN_STT_QUANT` or
  `VOICEXN_BOOST` is logged and the default kept, where it used to fail
  the load.
- `TtsWorker::spawn(config, playback)`: the recording folder moved into
  `TtsConfig::record`. `boost::find_vocab` and `VocabBooster::biases`
  moved into `SttConfig::for_platform`; `stt::quant` is gone.
- The desktop default is already `Q8_0`, as decision 5 says, before
  step 4 measures it.
- `android_main` still copies a build-time `VOICEXN_STT_QUANT` into the
  environment, where `for_platform` reads it.

## 2026-10-22 13:51 EDT — libvoicexn split, step 4: q4k on the desktop, by the owner's decision

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 4 and
decision 5, in `bd274ad` and `0750a89`.
- The owner (2026-10-22): "we won't be able to use q8 on mobile, so we
  should optimize getting q4k working". `SttQuant::DESKTOP` is `Q4k`, not
  the decision's provisional `Q8_0`. The A/B on the three plan 04 sentences
  (q4k mean 18 ms, q8 25–28 ms, transcripts tied) is logged; the one-minute
  recording of step 4 was not made, as the choice no longer rests on it.
- `scripts/fetch-stt.sh` builds q4k only by default, undoing step 3's
  `q4k q8`. `--quant` and `VOICEXN_STT_QUANT` still take q8_0 and q4_0.
- Boost tuning moves to another app of the owner's; `stt-moshi-xn-boost`
  here stays the example that shows it works.

## 2026-10-22 13:59 EDT — libvoicexn split, step 5: 0.1.0, and departures

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 5, in
`2db4042`, `1e4a0c1`, `66b8ed2`, `0d2d1da` and `3250b2b` (tag `v0.1.0`).
- `Cargo.toml` was already at 0.1.0, so no commit changes the version;
  the tag is on the `VERSIONS.md` commit, as the step says.
- The `VERSIONS.md` heading names the tag but not the commit
  (`## 0.1.0 — 2026-10-22 (tag v0.1.0)`), as libmorph, libmsdf and
  libcontrols do: a commit cannot name its own hash, and the tag pins it.
- Decision 6: `Stt`, `Step`, `FRAME`, `Segmenter`, `read_wav`, the VAD
  tuning constants, `boost::START`, `CONT` and `read_vocab` became
  `pub(crate)`. `STT_RATE`, `TTS_RATE` and `ECHO_HOLD_S` stay public,
  because the documentation of `Mic` and the ring is written in those terms.
- At the owner's request, the README is rewritten, not only its
  introduction: what the library does as STT/TTS, the example's rendering
  framework (libmorph, libmsdf, libcontrols) in its own section, the model
  downloads and the xn-moshi quantize step, vocabulary boosting, and the
  license. `LICENSE-APACHE` and `LICENSE-MIT` were added (libmorph's
  texts); every dependency is permissive, and the models are licensed
  separately (Phonon Apache-2.0, Kyutai STT CC-BY-4.0).

## 2026-10-22 14:15 EDT — libvoicexn split, step 6: the tty example, and departures

Plan `docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`, step 6, in
`0b3115b`.
- `voicexn-tty` always turns on `libvoicexn/stt-moshi-xn`, and forwards
  only `vad-earshot` and `stt-moshi-xn-boost`: without STT there is
  nothing for it to do. `--vocab` without the boost is logged and ignored.
- It opens the default devices in `f32`, the format CoreAudio gives; other
  sample formats are not converted.
- On an Apple M1, rayon's default of one thread per core (four of them slow)
  gave an 83.5 ms step on battery; `RAYON_NUM_THREADS=4` gave 26–36 ms.
  The library still sets no thread count (decision 4); choosing one in the
  examples is left to the q4k tuning plan.
- The minute of speech was a 55.7 s Phonon recording on the speaker, with
  the build that listens throughout. Phonon's replies mute the microphone
  (plan 02, decision 6), so not all of it was heard; dropped stayed 0.

## 2026-10-22 14:19 EDT — Mimi encoder-only plan

New planned work: load Kyutai STT's Mimi encoder from the GGUF that already
holds it, through a `Mimi::load_encoder` added to a fork of xn-moshi, so
`mimi.safetensors` (385 MB) is no longer read or pushed, and about 160 MB of
unused decoder weights leave memory. No crate on crates.io does this on xn.
Plan: `docs/design/20261022_06_MIMI_ENCODER_ONLY_PLAN.md` (Draft), steps 0
to 4.

## 2026-10-22 14:53 EDT — Mimi encoder-only, steps 1 to 4: departures

Plan `docs/design/20261022_06_MIMI_ENCODER_ONLY_PLAN.md`; `5d994ac` in
`tmzt/xn-moshi` (branch `mimi-encoder-only`, no pull request), and here
`b6480aa`, `6390507`, `f2a3023` and `32b8032` (tag `v0.2.0`).
- Step 1: `cargo fmt --check` cannot pass for the whole fork, because
  upstream's `examples/moshi.rs` is not formatted at `b5366b7`; it was
  left as it is, and only `src/mimi.rs` was checked. Clippy has no
  warnings before or after.
- Step 1: the test's tiny Mimi has two SEANet ratios (2, 2), not one, and
  its weights are not written out by hand: each name and shape is taken
  from the loaders' own "not found" and "shape mismatch" errors, filled
  with 0.01. `Mimi::load` now calls `load_encoder` and adds the decoder,
  reading the same tensors as before.
- Step 2: `mimi.safetensors` was not moved out of the shared model store,
  which other projects link to; `--model` pointed at a scratch folder
  linking the other three files instead.
- Step 3: `push-stt.sh` also deletes a `mimi.safetensors` left in the
  staging or app directory by an older push, and copies only the files it
  pushed.
- The S23's load was 15.3 s against 12.2 s before, inside the 13.9–21 s
  seen across plan 05; memory fell by 152 MB.

## 2026-10-22 16:27 EDT — Runtime vocabulary plan

New planned work, asked for by handlr's companion apps with the owner's
approval: replace the STT boost's phrase list on a running `SttWorker`
(`SttRequest::Vocabulary`, `SttEvent::Vocabulary`), encoded off the stt
thread and applied only between utterances, as libvoicexn 0.3.0.
Plan: `docs/design/20261022_07_STT_RUNTIME_VOCAB_PLAN.md` (Draft), five
steps. Not started: it is for the owner's review first.
