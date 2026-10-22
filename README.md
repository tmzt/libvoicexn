# libvoicexn

Realtime speech in and out, on the device: speech-to-text with Kyutai's
[stt-1b-en_fr](https://huggingface.co/kyutai/stt-1b-en_fr-candle) through
[xn-moshi](https://github.com/gradium-ai/xn-moshi), and text-to-speech with
Gradium's [Phonon 31m](https://huggingface.co/gradium/phonon-31m-en) through
[`ptts`](https://github.com/gradium-ai/xn-ptts). Both run on the CPU on
[xn](https://github.com/LaurentMazare/xn), in pure Rust and without ONNX
Runtime, on the desktop and on Android. On a Galaxy S23 Ultra the 4-bit
(q4k) STT model steps in 49–66 ms against its 80 ms frame, and Phonon's
first audio comes 30–80 ms after it is asked.

The library holds the engine and no UI:

- **`tts`**: a `TtsWorker` thread that loads Phonon, takes `Speak` and
  `Stop` requests, and streams 24 kHz audio into the playback ring as it
  is made.
- **`stt`**: an `SttWorker` thread that steps the STT model on each 80 ms
  frame of the microphone, and reports partial text, ended utterances and
  its own speed.
- **`capture`** and **`playback`**: the realtime ends. `Mic::push` takes
  the input callback's interleaved samples, downmixes, resamples to 24 kHz
  and mutes itself while the app is speaking; `Consumer::fill` feeds the
  output callback. Neither blocks or allocates on the audio thread.
- **`vad`** (`vad-earshot`): tap to talk. The pure-Rust
  [earshot](https://crates.io/crates/earshot) voice activity detector
  opens and closes a gate in front of the STT model, which is idle between
  turns.
- **`boost`** (`stt-moshi-xn-boost`): biases recognition toward a list of
  words the model does not know, such as product names, encoded by a
  SentencePiece tokenizer written in Rust (`spm`).
- **`config`**: `SttConfig` and `TtsConfig`, with `for_platform`
  defaults.
- **`morph`** (`morph`): adapters from the audio ends to a
  [libmorph](https://github.com/tmzt/libmorph) `Sink` and `Source`. The
  core does not depend on libmorph.

| Feature | What it adds |
|---|---|
| (none) | Text-to-speech |
| `stt-moshi-xn` | Speech-to-text |
| `vad-earshot` | Tap to talk, gated by earshot (implies `stt-moshi-xn`) |
| `stt-moshi-xn-boost` | Vocabulary boosting (implies `stt-moshi-xn`) |
| `morph` | libmorph `Sink` and `Source` adapters |

```rust
use libvoicexn::capture::Mic;
use libvoicexn::config::{SttConfig, TtsConfig};
use libvoicexn::playback;
use libvoicexn::stt::{SttEvent, SttWorker};
use libvoicexn::tts::{TtsRequest, TtsWorker};

// Two seconds of output at the device's rate, and a microphone that is
// muted while that ring is playing.
let (producer, consumer) = playback::ring(2 * out_rate as usize, out_channels, out_rate);
let (mic, capture) = Mic::new(in_rate, in_channels, consumer.speaking(), 4);
// Input callback: mic.push(samples). Output callback: consumer.fill(out).

let tts = TtsWorker::spawn(TtsConfig::for_platform("models/phonon-31m-en"), Some(producer));
let stt = SttWorker::spawn(SttConfig::for_platform("models/stt-1b-en_fr"), capture);
loop {
    for event in stt.poll() {
        if let SttEvent::Utterance(text) = event {
            tts.send(TtsRequest::Speak { text, voice: None });
        }
    }
    // ...
}
```

## The example: voicexn

[`examples/voicexn`](examples/voicexn) is a small app that listens, shows
what it hears and says it back with Phonon. It runs on Android, and on the
desktop, where it also has `--say "<text>"` (one utterance to
`out/tts/<unix ms>.wav`) and `--transcribe <wav>`.

Its screen and audio come from the rendering framework, which the library
does not need: [libmorph](https://github.com/tmzt/libmorph) for the window,
touch, microphone and speaker on Android and the desktop,
[libmsdf](https://github.com/tmzt/libmsdf) for the text, drawn as
multi-channel signed distance fields on the GPU, and
[libcontrols](https://github.com/tmzt/libcontrols) for the lines on screen.
It uses the library's `morph` feature to plug `Mic` and `Consumer` into
libmorph's audio.

## Models

The models are not in the repository. The scripts download them into a
shared store, `$COMMON_MODELS` (default `~/src/common-models`), and link
them into `models/`. Downloads are pinned to a revision, and a rerun only
fetches what is missing.

### Phonon 31m (text-to-speech)

```sh
scripts/fetch-model.sh
```

This fetches `config.json`, `tokenizer.json`, the q8 weights
`model.q8.gguf` and the default voice from
[gradium/phonon-31m-en](https://huggingface.co/gradium/phonon-31m-en), and
the voices listed in `scripts/voices.tsv` from
[gradium/phonon-voices](https://huggingface.co/gradium/phonon-voices).
Phonon is published already quantized; there is no step to run.

### Kyutai STT (speech-to-text)

```sh
scripts/fetch-stt.sh                               # q4k
STT_QUANTS="q4k q8_0 q4_0" scripts/fetch-stt.sh    # other quantizations as well
```

This fetches `config.json`, `tokenizer.model`, `mimi.safetensors` and the
full-precision `model.safetensors` (1.8 GB) from
[kyutai/stt-1b-en_fr-candle](https://huggingface.co/kyutai/stt-1b-en_fr-candle),
then quantizes the language model. Quantizing is done by xn-moshi's
`quantize` example, which the script clones at a pinned revision into
`target/xn-moshi` and runs as:

```sh
cargo run --release --manifest-path target/xn-moshi/Cargo.toml --example quantize -- \
    asr ~/src/common-models/stt-1b-en_fr model.q4k.gguf --quant q4k
```

The output holds the quantized LM and Mimi's encoder in one file:

| Quantization | File | Size | Used |
|---|---|---|---|
| `q4k` | `model.q4k.gguf` | 974 MB | The default on Android and the desktop |
| `q8_0` | `model.q8.gguf` | 1.3 GB | `--quant q8` |
| `q4_0` | `model.q4_0.gguf` | 974 MB | `--quant q4_0` |

q8 is too slow for the phone (82–133 ms steps against 80 ms on the S23),
so q4k is the default everywhere and the desktop runs what the phone runs.

### Licenses of the models

Phonon 31m and its voices are Apache-2.0. Kyutai's stt-1b-en_fr, including
Mimi, is [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/): an app
that ships it must credit Kyutai. The example's font is Roboto
(Apache-2.0), bundled by libmsdf.

## Vocabulary boosting

With `stt-moshi-xn-boost`, the STT model is biased toward a list of
phrases, one per line, with `#` comments. The library splits each phrase
into the model's own SentencePiece tokens, as written and capitalized,
and raises those tokens' scores while the model writes: `start` on a
phrase's first token, and `cont` on the next token of a phrase it has
begun. It is a demonstration that boosting works on xn-moshi, not a tuned
list: on the plan 04 A/B, the default biases got `Libgenai` right where
plain decoding did not, and stronger ones fixed more names but cut
sentences short.

```sh
scripts/vocab-lib.sh          # writes vocab.txt: the lib* folders in ~/src
$EDITOR vocab.txt             # or any list of your own
```

The list is found in this order: `VOICEXN_VOCAB`, `vocab.txt` in the
working directory (the desktop), then `vocab.txt` next to the model folder
(Android's data directory). The default biases are `start` 1.5 and `cont`
8; `VOICEXN_BOOST=start,cont` overrides them. Too much bias leaves the
model inside a word that never ends: at 2/16 a sentence of names stopped
halfway. The A/B is in [CHANGES.md](CHANGES.md) under plan 20261022_04.

## Running

On the desktop, from the repository root, so `models/` and `vocab.txt`
resolve:

```sh
cargo run --release -p voicexn -- --say "Hello from voicexn."
cargo run --release -p voicexn --features "vad-earshot stt-moshi-xn-boost" -- \
    --transcribe speech.wav [--quant q4k|q4_0|q8]
```

On Android (arm64; needs the `aarch64-linux-android` Rust target, an
Android SDK with an NDK, Gradle 9 and a JDK 17 or 21):

```sh
scripts/android.sh --install -- --features "vad-earshot stt-moshi-xn-boost"
scripts/push-model.sh         # Phonon into the app's data directory
scripts/push-stt.sh           # model.q4k.gguf and the rest, about 1.4 GB
scripts/push-vocab.sh         # vocab.txt, for the boost
```

The push scripts use `adb` and `run-as`, so the debuggable APK must be
installed first. Tap the screen to talk; the app says back what it heard.

### Overrides

| Variable | Read by | Effect |
|---|---|---|
| `VOICEXN_STT_QUANT` | `SttConfig::for_platform` | `q4k`, `q4_0` or `q8_0`. On Android, set at build time. |
| `VOICEXN_VOCAB` | `SttConfig::for_platform` | The phrase list for the boost |
| `VOICEXN_BOOST` | `SttConfig::for_platform` | `start,cont` biases for the boost |
| `VOICEXN_THREADS` | voicexn's `android_main`, at build time | rayon threads on Android (default 4 with STT, 2 without) |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option. The models are licensed separately (above).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

## Development

Contributor and agent rules are in [AGENTS.md](AGENTS.md); planned work is
logged in [CHANGES.md](CHANGES.md), and versions in
[VERSIONS.md](VERSIONS.md).

## Architectural decisions

| Document | Date | Status | Decision |
|---|---|---|---|
| [Phonon 31m text-to-speech on Android](docs/design/20261022_01_PHONON_TTS_ON_ANDROID_PLAN.md) | 2026-10-22 | In implementation | One libmorph app calls `ptts` (xn, CPU, q8) on its own thread and streams 24 kHz chunks, resampled, into a libmorph audio source, behind a libcontrols screen of preset phrases; voices are the web demo's short list. The checkpoint is pushed over adb. The owner asked to start; step 1 is the first implementing commit. STT is deferred to its own plan. |
| [Speech-to-text with Kyutai STT on xn-moshi](docs/design/20261022_02_STT_MOSHI_XN_PLAN.md) | 2026-10-22 | In implementation | Behind a `stt-moshi-xn` feature, a `stt` thread runs Kyutai's `stt-1b-en_fr` (q8) through `xn-moshi` on microphone audio captured by libmorph, decoded by a small Rust SentencePiece reader, half-duplex with the Phonon TTS. Started at the owner's request; the phone's speed is measured in step 2 before any screen work. |
| [Tap to talk, gated by earshot voice activity detection](docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md) | 2026-10-22 | In implementation | Behind a `vad-earshot` feature, a tap arms listening, and the pure-Rust `earshot` VAD (no ONNX) opens and closes the gate. STT steps only while it is open, and Phonon repeats the utterance when it closes. Started at the owner's request. |
| [Boosting a vocabulary list in Kyutai STT](docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md) | 2026-10-22 | In implementation | Behind `stt-moshi-xn-boost`, a `TokenBooster` biases the STT model toward the token sequences of a phrase list (first: the `lib*` folders in `~/src`), encoded by a new Rust SentencePiece encoder. Started at the owner's request, with a desktop A/B before the phone. |
| [libvoicexn, a speech library with voicexn as its demo](docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md) | 2026-10-22 | Implemented | The repository becomes `libvoicexn`: the engine as a library with no libmorph in its core, per-platform quant configs (q4k on Android, q8 on desktop if measured better), version 0.1.0, and voicexn plus a `cpal` terminal version as examples. Implemented: 0.1.0 is tagged, and both examples ran; the desktop default became q4k by the owner's decision. |
| [Mimi's encoder from the STT GGUF, without mimi.safetensors](docs/design/20261022_06_MIMI_ENCODER_ONLY_PLAN.md) | 2026-10-22 | In implementation | No crate does it on xn, so a small `Mimi::load_encoder` goes into a fork of xn-moshi (offered upstream), and `Stt::load` reads Mimi from the GGUF that already holds it: 385 MB less to push and about 160 MB less memory, as libvoicexn 0.2.0. Approved by the owner, with the fork made and no upstream pull request. |
