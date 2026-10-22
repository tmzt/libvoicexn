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
