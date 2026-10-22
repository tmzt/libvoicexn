# voicexn

A minimal realtime speech example on Android: a [libmorph](https://github.com/tmzt/libmorph)
app drawn with [libmsdf](https://github.com/tmzt/libmsdf) and
[libcontrols](https://github.com/tmzt/libcontrols) that speaks with Gradium's
Phonon 31m through [`ptts`](https://github.com/gradium-ai/xn-ptts) on
[xn](https://github.com/LaurentMazare/xn), on the CPU and without ONNX Runtime.
Text-to-speech comes first; speech-to-text follows in a later plan.

## Development

Contributor and agent rules are in [AGENTS.md](AGENTS.md); planned work is
logged in [CHANGES.md](CHANGES.md).

## Architectural decisions

| Document | Date | Status | Decision |
|---|---|---|---|
| [Phonon 31m text-to-speech on Android](docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md) | 2026-10-22 | In implementation | One libmorph app calls `ptts` (xn, CPU, q8) on its own thread and streams 24 kHz chunks, resampled, into a libmorph audio source, behind a libcontrols screen of preset phrases; voices are the web demo's short list. The checkpoint is pushed over adb. The owner asked to start; step 1 is the first implementing commit. STT is deferred to its own plan. |
| [Speech-to-text with Kyutai STT on xn-moshi](docs/design/20261022_02_STT_MOSHI_XN_PLAN.md) | 2026-10-22 | In implementation | Behind a `stt-moshi-xn` feature, a `stt` thread runs Kyutai's `stt-1b-en_fr` (q8) through `xn-moshi` on microphone audio captured by libmorph, decoded by a small Rust SentencePiece reader, half-duplex with the Phonon TTS. Started at the owner's request; the phone's speed is measured in step 2 before any screen work. |
| [Tap to talk, gated by earshot voice activity detection](docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md) | 2026-10-22 | Draft | Behind a `vad-earshot` feature, a tap arms listening, and the pure-Rust `earshot` VAD (no ONNX) opens and closes the gate. STT steps only while it is open, and Phonon repeats the utterance when it closes. Drafted at the owner's request. |
