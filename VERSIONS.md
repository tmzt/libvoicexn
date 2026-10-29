# VERSIONS

Each change of `libvoicexn`'s version and the public API changes it
carries (AGENTS.md, Rule 5). Internal changes are not listed, and there is
no entry without a version change. The examples are `publish = false` and
are not versioned. Entries are appended, oldest first, and never edited.

## 0.1.0 — 2026-10-22 (tag v0.1.0)

The first version as a library
(`docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md`), with the engine
built under plans 20261022_01 to 04. Its public API is what the examples
use (decision 6); everything else is crate-private.

- Features: `stt-moshi-xn` (speech-to-text), `vad-earshot` (tap to talk,
  implies `stt-moshi-xn`), `stt-moshi-xn-boost` (vocabulary boost,
  implies `stt-moshi-xn`) and `morph` (the libmorph adapters). With none,
  text-to-speech only.
- `config`: `TtsConfig { model_dir, quant: ptts::synth::Quant, record }`
  with `TtsConfig::for_platform(model_dir)` (q8, nothing recorded).
  With `stt-moshi-xn`: `SttQuant` (`Q4k`, `Q4_0`, `Q8_0`; `ANDROID`,
  `DESKTOP`, both `Q4k`; `parse`, `for_platform`, `file`), and
  `SttConfig { model_dir, quant }`, plus `vocab: Option<PathBuf>` and
  `boost: (f32, f32)` with `stt-moshi-xn-boost`, with
  `SttConfig::for_platform(model_dir)`, which reads `VOICEXN_STT_QUANT`,
  `VOICEXN_VOCAB` and `VOICEXN_BOOST`.
- `tts`: `TtsWorker::spawn(TtsConfig, Option<playback::Producer>)`,
  `send(TtsRequest)`, `poll()` and `wait()`; `TtsRequest` (`Speak { text,
  voice }`, `Stop`); `TtsEvent` (`Loaded { voices, load_ms }`,
  `LoadFailed`, `Started { id }`, `FirstAudio { id, ms }`, `Finished { id,
  audio_ms, synth_ms }`, `Stopped { id }`, `Failed { id, error }`).
- `playback`: `TTS_RATE`; `ring(capacity_frames, channels, out_rate) ->
  (Producer, Consumer)`; `Producer::push`, `flush` and `reset`;
  `Consumer::fill(&mut [f32])` and `speaking() -> Arc<AtomicBool>`;
  `Resampler::new`, `process` and `reset`.
- With `stt-moshi-xn`, `stt`: `STT_RATE`, `ECHO_HOLD_S`; `Capture { ring,
  stats }` and `CaptureStats::take() -> (peak, dropped)`;
  `SttWorker::spawn(SttConfig, Capture)`, `poll()` and, with
  `vad-earshot`, `send(SttRequest)`; `SttEvent` (`Loaded { load_ms }`,
  `LoadFailed`, `Partial`, `Utterance`, `Speed { step_ms, dropped }`, and
  `Gate(GateState)` with `vad-earshot`); `SttRequest` (`Arm`, `Close`);
  `transcribe_file(&SttConfig, &Path) -> Result<String, String>`.
- With `stt-moshi-xn`, `capture`: `Mic::new(in_rate, channels, speaking,
  seconds) -> (Mic, stt::Capture)` and `Mic::push(&[f32])`.
- With `stt-moshi-xn`, `spm`: `Decoder::open`, `from_bytes`, `encode`,
  `decode`, `starts_word`, `len` and `is_empty`; `spm::Error`.
- With `vad-earshot`, `vad`: `Gate` (`new`, `state`, `listening`, `arm`,
  `close`, `finish`, `push`) and `GateState` (`Idle`, `Armed`, `Open`,
  `Closing`).
- With `stt-moshi-xn-boost`, `boost`: `VocabBooster::new`,
  `from_phrases(&Decoder, &[String], start, cont)`, `len` and `is_empty`,
  an `xn_moshi::asr::TokenBooster`.
- With `morph`: `morph::source(Consumer) -> libmorph::Source`, and with
  `stt-moshi-xn`, `morph::sink(Mic) -> libmorph::Sink` and
  `morph::capture(CaptureFormat, speaking, seconds) -> (Sink, Capture)`.

## 0.2.0 — 2026-10-22 (tag v0.2.0)

Mimi's encoder from the STT GGUF
(`docs/design/20261022_06_MIMI_ENCODER_ONLY_PLAN.md`).

- **Breaking:** xn-moshi 0.2.6 from crates.io → the fork
  `github.com/tmzt/xn-moshi` at `5d994ac` (0.2.6 with
  `Mimi::load_encoder`). `boost::VocabBooster` implements
  `xn_moshi::asr::TokenBooster` from that source, so an app that also
  depends on crates.io's xn-moshi sees a different trait.
- Changed: `stt::SttWorker::spawn` and `stt::transcribe_file` no longer read
  `mimi.safetensors`; the model folder needs only `model.<quant>.gguf`,
  `config.json` and `tokenizer.model`. A `mimi.safetensors` there is
  ignored.

## 0.3.0 — 2026-10-29 (tag v0.3.0)

Changing the STT vocabulary boost while the model runs
(`docs/design/20261022_07_STT_RUNTIME_VOCAB_PLAN.md`).

- **Breaking:** `stt::SttRequest` is no longer `Copy` (it is `Clone`), and
  with `stt-moshi-xn-boost` gains `Vocabulary(Vec<String>)`: it replaces
  the boosted phrases of a running `SttWorker` from the next utterance,
  and an empty list turns the boost off. `SttRequest` and
  `SttWorker::send` now exist with `vad-earshot` or `stt-moshi-xn-boost`.
- **Breaking:** with `stt-moshi-xn-boost`, `stt::SttEvent` gains
  `Vocabulary { phrases, sequences }`, sent when a list takes effect: at
  load from `SttConfig::vocab`, and after each `SttRequest::Vocabulary`.
- Added, with `stt-moshi-xn-boost`: `boost::BoostTable` (`new`,
  `from_phrases(&Decoder, &[String])`, `len`, `is_empty`),
  `boost::BoostSlot`, and `VocabBooster::with_slot(self) -> (Self,
  BoostSlot)`.

## 0.4.0 — 2026-10-29 (tag v0.4.0)

A speech session in libvoicexn, with no libmorph and cpal as an option
(`docs/design/20261029_01_SESSION_PLAN.md`). The generic speech plumbing of
flankr's `flankr-audio` 0.2.0 moves here.

- **Breaking:** the `morph` feature, the `morph` module (`morph::sink`,
  `morph::source`, `morph::capture`) and the libmorph dependency are
  removed. A libmorph app joins the audio ends to libmorph with two
  closures on its own libmorph revision: a `Sink` calling `Mic::push`
  and a `Source` calling `Playback::fill` or `Consumer::fill` (README,
  "Using libvoicexn in a libmorph app").
- Added, in the core: `playback::Playback` (`new(Consumer,
  Arc<AtomicU32>)`, `with_cue(Vec<f32>, Arc<AtomicU32>)`, `speaking`,
  `fill`), which drains the ring whenever its flush count moves, zeroes
  what the ring does not cover, and mixes a mono cue over every channel;
  and `playback::chime(rate) -> Vec<f32>`.
- Added, with `session` (implies `stt-moshi-xn`): the `session` module.
  - `Format { rate, channels }`.
  - `SessionConfig { stt, tts, voice, vocab_file, echo_mute }` and
    `SessionConfig::new(models)`.
  - `Action` (`Speak`, `Stop`, `Arm`, `Close`, `Vocabulary`, `Cue`).
  - `SessionEvent` (`Utterance`, `Partial`, `Listening`, `Voice`,
    `Speaking`, `SttLoaded`, `SttLoadFailed`, `TtsLoaded`,
    `TtsLoadFailed`, `Speed`, `Spoken`, `SpeakFailed`, `VocabularySaved`,
    `VocabularyFailed`; `Gate` with `vad-earshot`; `Vocabulary` with
    `stt-moshi-xn-boost`).
  - `Session::spawn(SessionConfig, Format, Format) -> (Session, Mic,
    Playback)`, `act`, `poll`, `set_voice`, `voice` and `vocab_file`.
  - `session::pump`: `Pump` (`new`, `on_action`, `on_stt`, `on_tts`,
    `tick`), `tts_request`, `stt_request`, `gate_listening` (with
    `vad-earshot`), `vocab_text` and `write_vocab`.
  - `Action::Vocabulary` sends `SttRequest::Vocabulary` to the running
    model with `stt-moshi-xn-boost`, and writes `vocab_file` when one is
    set; a `vocab_file` that exists at spawn is read in place of
    `SttConfig::vocab`.
- Added, with `cpal` (implies `session`; cpal 0.18.2):
  `Session::open_default(SessionConfig)` and
  `Session::open_default_output(SessionConfig, Format) -> (Session, Mic)`.
- Changed, scripts: `push-model.sh`, `push-stt.sh` and `push-vocab.sh`
  take `APP_ID`, `VOICEXN_MODELS` and `STAGE`, and follow links in a
  shared models folder.
