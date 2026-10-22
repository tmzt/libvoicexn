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
