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
