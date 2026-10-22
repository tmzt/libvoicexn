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
