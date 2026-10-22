---
title: Speech-to-text with Kyutai STT on xn-moshi
date: 2026-10-22
status: In implementation
covers:
  - A cargo feature `stt-moshi-xn` on the voicexn crate. When it is off,
    the build and the dependency graph are what plan 20261022_01 gives.
  - Live microphone capture through libmorph, resampled to 24 kHz mono
    for Mimi
  - Streaming transcription with Kyutai's `stt-1b-en_fr` through the
    `xn-moshi` crate, on the CPU with q8 weights, on its own thread, with
    no ONNX Runtime
  - A pure-Rust SentencePiece decoder for the transcript (no C++
    `sentencepiece`)
  - Half-duplex with the TTS of plan 20261022_01: the microphone is
    replaced by silence while voicexn is speaking
  - The transcript on screen, and a tap that speaks it back with Phonon
  - Not covered: barge-in, echo cancellation beyond Android's input
    presets, voice activity detection, languages other than English
    (the model also does French).
sources:
  - Owner's request (2026-10-22): "add stt-moshi-xn feature on new branch
    feat-moshi-xn".
  - gradium-ai/xn-moshi at b5366b7 (2026-09-22), crate `xn-moshi` 0.2.6 on
    crates.io (MIT/Apache-2.0, `xn = "0.2.5"`, which unifies with the xn
    0.2.12 that `ptts` brings): `asr::Asr::load(mimi, lm, config,
    asr_delay_in_tokens, temperature, dev)`, `Asr::init_state(batch)`,
    `AsrState::step_pcm(&StreamTensor, &StreamMask, f)` on 1920-sample
    frames at 24 kHz (80 ms, 12.5 Hz), returning `StepResult { words:
    Vec<AsrWord> }` with `AsrWord::Word { tokens, start_time, .. }`,
    `EndWord { stop_time, .. }` and `EndOfStream`; `examples/moshi.rs`
    `run_asr` (two frames of leading silence, the ASR delay's worth of
    trailing silence, token 3 put back between words before SentencePiece
    decoding); `examples/quantize.rs asr <in> <out> --quant q8_0`, which
    quantizes the transformer linears and leaves out `mimi.decoder`.
  - Hugging Face `kyutai/stt-1b-en_fr-candle` at `095e38f6`, CC-BY-4.0:
    `model.safetensors` (1978.5 MB), `mimi-pytorch-e351c8d8@125.safetensors`
    (384.6 MB), `tokenizer_en_fr_audio_8000.model`, `config.json` (`dim`
    2048, 16 layers, `n_q` 32, `text_card` 8000, `stt_config.
    audio_delay_seconds` 0.5). `kyutai/stt-2.6b-en-candle` (5.2 GB) is
    the only other candle checkpoint and is too large for the phone.
  - libmorph 0.3.0: `AudioSetup { capture, capture_mode, .. }`,
    `CaptureMode::VoiceRecognition`, `Audio::add_sink` with `Sink = Box<dyn
    FnMut(&Captured) + Send>` on the input thread, which must not block,
    `Captured { samples, frame, time }`, `CaptureFormat`,
    `MorphApp::on_capture_ready`, and RECORD_AUDIO asked for after `init`
    when the manifest declares it.
related:
  - docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md (the app, the
    playback ring and the tts thread this builds on)
  - libmorph docs/design/20261022_01_AUDIO_CAPTURE_PLAN.md
---

# Speech-to-text with Kyutai STT on xn-moshi

## 1. Goal

With `--features stt-moshi-xn`, voicexn listens through the microphone
and shows what is said, word by word, while it is being said. A tap
speaks the transcript back with Phonon. Both models run on the phone's
CPU through xn.

The main risk is speed. The smallest STT checkpoint xn-moshi loads has
about 1B parameters, and it must take each 80 ms frame in under 80 ms on
the S23. Step 2 measures that on the desktop and the phone before any
screen work. If the phone cannot keep up, the plan stops there and the
result goes to the owner.

## 2. Decisions

1. **A feature, off by default.** `stt-moshi-xn = ["dep:xn-moshi"]`. All
   STT code is under `#[cfg(feature = "stt-moshi-xn")]`. When it is on,
   `build.rs` adds `android.permission.RECORD_AUDIO` to the manifest (it
   reads `CARGO_FEATURE_STT_MOSHI_XN`).
2. **`xn-moshi` 0.2.6 from crates.io**, CPU only, with no backend
   features. There is one `xn` in the graph, shared with `ptts`.
3. **Model: `kyutai/stt-1b-en_fr-candle` at `095e38f6`**, with the LM
   quantized once to q8_0 GGUF on the desktop by xn-moshi's own `quantize`
   example. Like Phonon, the model is pushed over adb and is not in the APK.
   Its CC-BY-4.0 licence asks for attribution, which goes in the README.
4. **One `stt` thread owns the model**, like the `tts` thread. It reads
   24 kHz mono from a second `rtrb` ring that the capture sink fills, steps
   the model one 1920-sample frame at a time, and sends `SttEvent`s.
5. **The capture sink only converts and pushes.** It averages channels to
   mono, resamples to 24 kHz with `playback::Resampler`, and pushes. While
   it is full it drops samples and counts them, and never waits.
6. **Half-duplex.** While the playback ring holds audio, the sink pushes
   zeros in place of the microphone, so the model's clock keeps running
   but it does not hear voicexn speak. `Consumer::fill` sets a shared
   `speaking` flag.
7. **SentencePiece decode in Rust.** The `.model` file is a protobuf whose
   field 1 repeats `SentencePiece { piece: string = 1, .. }`. The decoder
   reads only that, maps ids to pieces, turns `▁` into a space and
   `<0xNN>` byte pieces into bytes. It does not encode.
8. **Capture mode `VoiceRecognition`**, AAudio's preset for ASR.

## 3. Layout

New files, all behind the feature except the scripts:

```
src/stt.rs            # SttWorker: thread, ring, events
src/spm.rs            # SentencePiece piece table and decode
scripts/fetch-stt.sh  # download at 095e38f6, quantize to models/stt-1b-en_fr/
scripts/push-stt.sh   # adb push + run-as copy, like push-model.sh
```

`models/stt-1b-en_fr/` holds `model.q8.gguf`, `mimi.safetensors`,
`tokenizer.model` and `config.json`.

```rust
// stt.rs
pub enum SttEvent {
    Loaded { load_ms: u32 },
    LoadFailed(String),
    /// A word as it is recognised, decoded with the words before it.
    Word { text: String, start_s: f64 },
    /// Per 25 frames (2 s): mean step time against the 80 ms budget.
    Speed { step_ms: f32, dropped: u64 },
}
pub struct SttWorker { /* rx: Receiver<SttEvent> */ }
impl SttWorker {
    pub fn spawn(model_dir: PathBuf, input: rtrb::Consumer<f32>) -> Self;
    pub fn poll(&self) -> impl Iterator<Item = SttEvent> + '_;
}
pub fn capture_sink(format: libmorph::CaptureFormat, speaking: Arc<AtomicBool>,
                    ring: rtrb::Producer<f32>) -> libmorph::Sink;
```

## 4. Steps

1. **The feature and capture.** The feature, the dependency, the
   manifest permission, `AudioSetup.capture` with `VoiceRecognition`, the
   capture sink and its ring, and a log line every 2 s with the input level
   and the dropped-sample count. No model yet.
   **Done when:** `cargo tree -e normal` without the feature has no
   `xn-moshi`; with it, the S23 asks for the microphone, and the log shows
   the capture format and a level that rises when someone speaks.

2. **The model, and its speed.** `scripts/fetch-stt.sh` downloads the
   pinned checkpoint, quantizes the LM to q8_0 and lays out
   `models/stt-1b-en_fr/`. `src/spm.rs` with unit tests on a piece table
   built in the test. A desktop `--transcribe <wav>` runs `run_asr`'s
   sequence (leading and trailing silence, token 3 between words) and logs
   the text, the load time and the mean step time.
   **Done when:** `--transcribe` on the step-2 WAV of plan 20261022_01
   ("Phonon turns text into natural speech…") prints that sentence; the
   mean step time is logged for the M4; and the same file, run on the S23
   through a debug hook (`--transcribe` read from the app's arguments
   there), logs its step time. If the S23 step time is not under 80 ms
   with 2–4 threads, stop and report.

3. **Live transcription.** The `stt` thread reads the capture ring, and
   `App` shows the transcript under the status line, with the last 3
   lines kept. `Speed` updates the status line.
   **Done when:** on the S23, a spoken sentence appears word by word
   within about a second of each word, and `dropped` stays 0 for a minute
   of speech.

4. **Speak it back.** A tap speaks the transcript with Phonon and clears
   it. Half-duplex as in decision 6.
   **Done when:** on the S23, saying a sentence and tapping plays it back
   in the default voice, and the playback does not show up in the next
   transcript.

## 5. Open questions

1. Is the CC-BY-4.0 model licence acceptable for this example? It needs
   attribution, not payment.
2. Should step 4 keep the tap, or speak each sentence back automatically
   after a pause?

## Amendment — 2026-10-22 09:17 EDT: implementation has started; the document is now immutable

The owner asked for the feature to be added on `feat-moshi-xn`. Status
moves from `Draft` to `In implementation` in the commit that implements
step 1. From here this document changes only on the owner's explicit
instruction (AGENTS.md Rule 1.5); departures are recorded in
`CHANGES.md`.
