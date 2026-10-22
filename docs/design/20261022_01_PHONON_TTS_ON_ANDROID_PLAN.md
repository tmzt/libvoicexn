---
title: Phonon 31m text-to-speech on Android
date: 2026-10-22
status: Draft
covers:
  - The voicexn application crate: one libmorph app that runs on the
    desktop and as an Android `NativeActivity`
  - Text-to-speech with Gradium's Phonon 31m (English, Apache-2.0)
    through the `ptts` crate on xn, on the CPU, with no ONNX Runtime
  - Streaming the 24 kHz synthesis into libmorph's audio output as it is
    generated, resampled to the device rate
  - A small screen drawn with libmsdf and libcontrols: preset phrases, a
    voice picker, Stop, and a status line with timings
  - Getting the checkpoint onto the phone, and the build flags xn needs
    on arm64
  - Not covered: speech-to-text. It is the next plan (section 6).
sources:
  - Owner's request (2026-10-22): "build a minimal stt/tts realtime
    example with libmorph/libmsdf maybe libcontrols (if needed) and
    phonon 31m running on Android with xn (no onnx)", and the answers
    that xn is Laurent Mazare's inference runtime and that TTS comes
    first with Phonon 31m, which is newer and less restrictive.
  - gradium-ai/xn-ptts at 49b9033 (2026-10-21), workspace version 0.4.1:
    the `ptts` crate (`Checkpoint::resolve`, `checkpoint.builder(Lang::En)
    .device(DeviceKind::Cpu).build()`, `register_voices`, `tts.voices()`,
    `tts.stream(text)` yielding bounded PCM chunks, `SpeechOptions::voice`,
    `tts.session_default`), feature `hf` for the JSON tokenizer and `hub`
    for `hf://` downloads; `android/README.md` and `android/build.sh`
    (arm64 `+dotprod,+fp16`, `max-page-size=16384`, `RAYON_NUM_THREADS`,
    24 kHz mono `f32` in [-1, 1], load once, calls block).
  - LaurentMazare/xn at b50eef2 (2026-10-18); `ptts` takes `xn = "0.2.11"`
    from crates.io. Kernels are chosen at compile time.
  - Hugging Face `gradium/phonon-31m-en`: `model.q8.gguf` (64.5 MB),
    `config.json`, `tokenizer.json`, `default-voice.safetensors`; voices
    in `gradium/phonon-voices`. Weights, tokenizer and voices Apache-2.0.
    One 80 ms latent frame per step, Mimi decoder, 24 kHz mono.
  - libmorph 0.3.0 (`README.md`; `src/audio.rs`): `AudioSetup`,
    `Audio::format() -> Option<OutputFormat>`, `Audio::add_source` with
    `Source = Box<dyn FnMut(&mut [f32]) + Send>` filling interleaved
    `f32` at the output rate on the audio thread without blocking;
    `ctx.data_dir()`; `libmorph-build` Gradle generation; Android audio
    through libmobile.
  - libmsdf 0.4.2 (wgpu 30.0.1), libcontrols 0.5.0 (`Router`, `Button`,
    `ArrowLabel`, `Label`, `Panel`).
related:
  - libmorph docs/design/20261022_01_AUDIO_CAPTURE_PLAN.md (the capture
    path the STT plan will use)
  - libmorph docs/design/20261016_01_AUDIO_DEVICE_AND_SOURCES_PLAN.md
---

# Phonon 31m text-to-speech on Android

## 1. Goal

A tap on a phrase on an Android phone (the owner's Galaxy S23 first) is
heard from the phone's speaker as Phonon 31m generates it, starting
before the whole phrase is synthesised. The same app runs on the desktop
through libmorph's emulated phone, which is where each step is proven
first.

The model runs through `ptts`, which is built on xn. Nothing in the
dependency graph links ONNX Runtime (`ort`) or TensorFlow Lite.

## 2. Decisions

1. **`ptts` as a Rust dependency, not `libptts_ffi.so`.** voicexn is Rust
   already, so it calls `ptts` directly and the FFI/JNA layer in
   `xn-ptts/android` is not used. Dependency:
   `ptts = { version = "0.4.1", features = ["hf"] }`. `hub` is not
   enabled: the phone does not download from Hugging Face (decision 5).
2. **CPU only, q8.** `DeviceKind::Cpu`, `Quant::Q80`, loading
   `model.q8.gguf`. No Vulkan or QNN backend in this plan.
3. **One TTS thread owns the model.** `ptts` calls block and must come
   from one thread at a time. A `std::thread` named `tts` loads the
   checkpoint once and then serves requests from an `mpsc` channel; the
   render thread never calls `ptts`.
4. **Streaming into a libmorph `Source`.** The `tts` thread resamples
   each chunk from 24 000 Hz to `OutputFormat::sample_rate`, duplicates it
   to `OutputFormat::channels`, and pushes it into a single-producer,
   single-consumer ring buffer. The `Source` registered with
   `Audio::add_source` only pops from the ring (zeros when empty), so the
   audio thread never blocks or allocates.
5. **The checkpoint is not in the APK.** It is copied into
   `ctx.data_dir()/phonon-31m-en/` by a script over `adb` during
   development. Downloading on first run is left for later.
6. **libcontrols is used.** It is a thin layer over libmsdf, already
   carries buttons, an arrow picker and labels, and routes touch. Writing
   those by hand would be more code than depending on it.
7. **No free text input yet.** Preset phrases avoid the Android soft
   keyboard, which libmorph does not expose today.

## 3. Layout

```
voicexn/
  Cargo.toml          # one crate, [lib] crate-type = ["cdylib", "rlib"], plus a bin
  build.rs            # libmorph_build::gradle_project("dev.tmzt.voicexn")
  .cargo/config.toml  # Android arm64 rustflags (section 4)
  src/
    lib.rs            # android_main, App: MorphApp
    main.rs           # desktop entry: libmorph::run::<voicexn::App>()
    tts.rs            # TtsWorker: thread, channel, events
    playback.rs       # Ring, resampler, the libmorph Source
    ui.rs             # Router tree and status text
  scripts/
    fetch-model.sh    # hf download into models/phonon-31m-en (gitignored)
    push-model.sh     # adb push + run-as copy into the app's data dir
  out/                # gitignored evidence (AGENTS.md Rule 3.5)
```

Interfaces inside the crate:

```rust
// tts.rs
pub enum TtsRequest { Speak { text: String, voice: Option<String> }, Stop }
pub enum TtsEvent {
    Loaded { voices: Vec<String>, load_ms: u32 },
    LoadFailed(String),
    Started { id: u64 },
    FirstAudio { id: u64, ms: u32 },        // request to first chunk queued
    Finished { id: u64, audio_ms: u32, synth_ms: u32 }, // RTF = synth / audio
    Stopped { id: u64 },
}
pub struct TtsWorker { tx: Sender<TtsRequest>, rx: Receiver<TtsEvent> }
impl TtsWorker {
    pub fn spawn(model_dir: PathBuf, ring: playback::Producer) -> Self;
    pub fn send(&self, req: TtsRequest);
    pub fn poll(&self) -> impl Iterator<Item = TtsEvent> + '_;
}

// playback.rs
pub fn ring(capacity_frames: usize, channels: u16) -> (Producer, Consumer);
pub fn source(consumer: Consumer) -> libmorph::Source;
pub struct Resampler; // 24 kHz mono -> out rate, carries state across chunks
```

`Stop` is checked between chunks: the worker drops the rest of the
`SpeechStream` and clears the ring (the consumer sees a generation
counter change and discards what it holds).

The ring holds 2 s of output frames. When it is full, the worker waits
for space, which pauses generation (`ptts` bounds its own buffers).

## 4. Android build settings

`.cargo/config.toml` replaces the target-cpu default for arm64 with the
flags `xn-ptts/android/build.sh` uses:

```toml
[target.aarch64-linux-android]
rustflags = [
  "-C", "target-cpu=generic",
  "-C", "target-feature=+dotprod,+fp16",
  "-C", "link-arg=-Wl,-z,max-page-size=16384",
]
```

Only `arm64-v8a` is built. `RAYON_NUM_THREADS` is set to `2` with
`std::env::set_var` in `android_main` before the `tts` thread starts,
and to the core count on the desktop. The thread count is measured on
the S23 in step 5 and the result is recorded in the evidence log.

## 5. Steps

Each step is one commit to the `voicexn` crate unless it says otherwise
(AGENTS.md Rule 3).

1. **Scaffold.** `Cargo.toml` with libmorph (`mobile` on Android),
   libmorph-build, libmsdf with `font-apache-roboto-regular`, and
   libcontrols, all from git at pinned revisions. `App` clears the screen
   and draws "voicexn" with libmsdf. `build.rs` generates the Gradle
   project. `.gitignore` covers `target/`, `out/` and `models/`.
   **Done when:** `cargo run` shows the text in the emulated Galaxy S24;
   the APK built per libmorph's README installs and shows the same text
   on the S23; `cargo tree -d` lists exactly one `wgpu` and one `libmsdf`.

2. **Synthesis on the desktop.** `scripts/fetch-model.sh` downloads
   `gradium/phonon-31m-en` and the voices it lists into
   `models/phonon-31m-en/`. `tts.rs` loads it on the `tts` thread and
   handles `Speak` by collecting `tts.stream(text)` into a buffer. A
   `--say "<text>"` desktop argument writes `out/tts/<unix ms>.wav` with
   `ptts::wav::write_wav_file` and logs `load_ms`, `synth_ms`, `audio_ms`
   and their ratio. `cargo tree` shows no `ort`, `onnxruntime` or
   `tflitec`.
   **Done when:** `cargo run --release -- --say "Hello from voicexn."`
   writes a WAV file that plays as that sentence, and the log line gives
   the timings.

3. **Streaming playback.** `playback.rs` as in section 3. `App::init`
   creates the ring, registers the `Source`, and spawns the worker. A tap
   anywhere speaks a fixed sentence. A unit test pushes a 24 kHz sine
   through `Resampler` to 48 kHz and checks its frequency and that chunk
   boundaries leave no discontinuity.
   **Done when:** on the desktop a tap is heard starting while synthesis
   continues (`FirstAudio` is logged before `Finished`), with no clicks
   between chunks, and `cargo test -p voicexn` passes.

4. **Screen.** A libcontrols `Router` with a `Panel` holding four phrase
   `Button`s, an `ArrowLabel` of `voices`, a `Stop` button, and a `Label`
   status line: `loading…`, then `ready · load <n> ms`, then per phrase
   `first audio <n> ms · RTF <x>`. Layout keeps to `ctx.safe_area()`.
   **Done when:** on the desktop each phrase speaks in the chosen voice,
   Stop silences it within one 80 ms frame plus the output buffer, and a
   `--morph-script` run writes screenshots of the loading and ready
   states to `out/morph/`.

5. **On the phone.** `scripts/push-model.sh` pushes the model folder to
   `/data/local/tmp/voicexn/` and copies it with
   `adb shell run-as dev.tmzt.voicexn` into the data directory. The app
   shows `model missing: <path>` in the status line when it is not there.
   The thread count (1, 2, 3, 4) is compared by RTF.
   **Done when:** on the S23, a release APK loads the model, each phrase
   is heard from the speaker, the status line and logcat show first-audio
   time and an RTF below 1.0, and the measurements are appended to
   `out/tts/LOG.md`.

## 6. Next: speech-to-text

Out of scope here, written up as its own plan once this one is
implemented. The capture side exists: libmorph's `AudioSetup::capture`,
`Audio::add_sink`, and `CaptureMode` (Android's voice-recognition
preset). The open choice is the STT model and whether it runs on xn
today. That is a question for the owner, not settled here.

## 7. Open questions

1. Package name `dev.tmzt.voicexn`: is that right?
2. Which four preset phrases? Placeholders until then.
3. Should the voice picker list every voice in `gradium/phonon-voices`
   (about 100), or a fixed short list bundled with the push script?

## Amendment — 2026-10-22 08:50 EDT: the voice list, and Apple silicon builds

The owner answered open question 3 and noted that the desktop builds run on
Apple silicon. This amendment replaces open question 3 in section 7, adds to
step 2 in section 5, and adds to section 4.

**Voices.** The picker offers the short list from Gradium's web demo
(Hugging Face space `gradium/phonon` at `fcf37734`, `models.json`, entry
"Phonon 31M"): `default` (the checkpoint's `default-voice.safetensors`)
followed by the demo's 37 voices in its order, 12 American English, 13
British English and 12 characters. Their files come from
`gradium/phonon-voices` at `da6c9ec0d47be8df5c87c730a3aa97187f3229c3`, the
revision `ptts` 0.4.1 pins in `voice_catalog.rs`. `scripts/fetch-model.sh`
downloads each one to `models/phonon-31m-en/voices/<name>.safetensors`,
named by the demo's display name, so `register_voices` lists them under
those names. The list is kept in `scripts/voices.tsv` (name, locale, file).
The demo's English example sentence, "Phonon turns text into natural
speech, right on your device, with no cloud round trip.", is the first
preset phrase. Open questions 1 and 2 stay open for the rest.

**Apple silicon.** The desktop target `aarch64-apple-darwin` defaults to
`target-cpu=apple-m1`, which already has `neon`, `dotprod` and `fp16`,
so xn compiles the same q8 kernels on the Mac as on the phone with no
rustflags. `.cargo/config.toml` sets nothing for it. The timings logged in
step 2 are therefore a fair preview of the phone's kernels, but not of its
speed.
