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
