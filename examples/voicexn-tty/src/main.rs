//! libvoicexn in a terminal (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md,
//! decision 8): the default microphone into STT, each utterance printed as
//! `> text` and said back by Phonon on the default output. No window and no
//! libmorph: `cpal` opens the devices, `Mic::push` and `Consumer::fill` are
//! its callbacks.
//!
//! ```text
//! cargo run --release -p voicexn-tty [--features vad-earshot] -- \
//!     [--quant q4k|q4_0|q8] [--voice <name>] [--vocab <file>] [--input <wav>]
//! ```
//!
//! Enter does what a tap does in voicexn: with `vad-earshot` it arms the
//! gate, or closes it while it is listening; without, it repeats the last
//! utterance. With `stt-moshi-xn-boost`, a line `vocab: a, b, c` replaces
//! the boosted phrases, and `vocab:` alone clears them
//! (docs/design/20261022_07_STT_RUNTIME_VOCAB_PLAN.md, decision 6). End of
//! input (Ctrl-D) quits.
//!
//! `--input <wav>` hears the file instead of the default input device, at
//! real time from the moment STT has loaded, and quits 2 s after its end.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use libvoicexn::capture::Mic;
use libvoicexn::config::{SttConfig, SttQuant, TtsConfig};
use libvoicexn::playback;
use libvoicexn::stt::{SttEvent, SttWorker};
use libvoicexn::tts::{TtsEvent, TtsRequest, TtsWorker};
#[cfg(feature = "vad-earshot")]
use libvoicexn::{stt::SttRequest, vad::GateState};

/// How often the terminal is updated.
const TICK: Duration = Duration::from_millis(20);
/// How much of `--input` is pushed at a time.
const BLOCK: Duration = Duration::from_millis(20);
/// Silence before `--input`: two of Mimi's frames, as `transcribe_file`
/// puts before a file.
const LEAD: Duration = Duration::from_millis(160);
/// How long after the end of `--input` it quits.
const TAIL: Duration = Duration::from_secs(2);

const USAGE: &str =
    "usage: voicexn-tty [--quant q4k|q4_0|q8] [--voice <name>] [--vocab <file>] [--input <wav>]";

struct Args {
    quant: Option<SttQuant>,
    voice: Option<String>,
    vocab: Option<PathBuf>,
    input: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut parsed = Args {
        quant: None,
        voice: None,
        vocab: None,
        input: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--quant" => {
                let name = value()?;
                let quant = SttQuant::parse(&name);
                parsed.quant =
                    Some(quant.ok_or(format!("--quant {name:?}: one of q4k, q4_0, q8"))?);
            }
            "--voice" => parsed.voice = Some(value()?),
            "--vocab" => parsed.vocab = Some(PathBuf::from(value()?)),
            "--input" => parsed.input = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument {arg:?}\n{USAGE}")),
        }
    }
    Ok(parsed)
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// What the terminal and `--input` tell the main loop.
enum Input {
    /// Enter, or any line but `vocab:`.
    Enter,
    /// A `vocab:` line's comma-separated names.
    Vocab(Vec<String>),
    /// `--input` has ended, and [`TAIL`] has passed.
    FileEnd,
}

fn parse_line(line: &str) -> Input {
    match line.trim().strip_prefix("vocab:") {
        Some(names) => Input::Vocab(
            names
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect(),
        ),
        None => Input::Enter,
    }
}

fn run(args: Args) -> Result<(), String> {
    let host = cpal::default_host();
    let output = host.default_output_device().ok_or("no output device")?;
    let out_config = output.default_output_config().map_err(|e| e.to_string())?;
    let (out_rate, out_channels) = (out_config.sample_rate(), out_config.channels());
    log::info!("playback: {out_rate} Hz, {out_channels} channels");

    // Two seconds of output, and a microphone muted while it plays. A file
    // has no echo, so it is never muted.
    let (producer, mut consumer) = playback::ring(out_rate as usize * 2, out_channels, out_rate);
    let mut file = None;
    let mut in_stream = None;
    let capture = if let Some(path) = &args.input {
        let wav = read_wav(path).map_err(|e| format!("{}: {e}", path.display()))?;
        log::info!(
            "input: {}, {} Hz, {} channels, {:.1} s",
            path.display(),
            wav.rate,
            wav.channels,
            wav.samples.len() as f32 / wav.channels as f32 / wav.rate as f32
        );
        let never = Arc::new(AtomicBool::new(false));
        let (mic, capture) = Mic::new(wav.rate, wav.channels, never, 4);
        file = Some((mic, wav));
        capture
    } else {
        let input = host.default_input_device().ok_or("no input device")?;
        let in_config = input.default_input_config().map_err(|e| e.to_string())?;
        let (in_rate, in_channels) = (in_config.sample_rate(), in_config.channels());
        log::info!("capture: {in_rate} Hz, {in_channels} channels");
        let (mut mic, capture) = Mic::new(in_rate, in_channels, consumer.speaking(), 4);
        let stream = input
            .build_input_stream::<f32, _, _>(
                in_config.config(),
                move |samples: &[f32], _: &cpal::InputCallbackInfo| mic.push(samples),
                |e| log::error!("capture: {e}"),
                None,
            )
            .map_err(|e| format!("capture: {e}"))?;
        in_stream = Some(stream);
        capture
    };
    let out_stream = output
        .build_output_stream::<f32, _, _>(
            out_config.config(),
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| consumer.fill(out),
            |e| log::error!("playback: {e}"),
            None,
        )
        .map_err(|e| format!("playback: {e}"))?;
    out_stream.play().map_err(|e| format!("playback: {e}"))?;
    if let Some(stream) = &in_stream {
        stream.play().map_err(|e| format!("capture: {e}"))?;
    }

    let tts = TtsWorker::spawn(
        TtsConfig::for_platform("models/phonon-31m-en"),
        Some(producer),
    );
    let mut stt_config = SttConfig::for_platform("models/stt-1b-en_fr");
    if let Some(quant) = args.quant {
        stt_config.quant = quant;
    }
    if let Some(vocab) = args.vocab {
        #[cfg(feature = "stt-moshi-xn-boost")]
        {
            stt_config.vocab = Some(vocab);
        }
        #[cfg(not(feature = "stt-moshi-xn-boost"))]
        log::warn!(
            "--vocab {}: built without stt-moshi-xn-boost",
            vocab.display()
        );
    }
    log::info!(
        "stt: loading {} ({:?})",
        stt_config.model_dir.display(),
        stt_config.quant
    );
    let stt = SttWorker::spawn(stt_config, capture);

    // Lines and end of input, from a thread of their own. With `--input`,
    // the file's sender keeps the channel open past the end of stdin.
    let (keys, inputs) = mpsc::channel();
    let file_end = file.as_ref().map(|_| keys.clone());
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if keys.send(parse_line(&line)).is_err() {
                return;
            }
        }
    });
    let mut file = file.zip(file_end);

    let mut terminal = Terminal::default();
    #[cfg(not(feature = "vad-earshot"))]
    let mut last = None::<String>;
    #[cfg(feature = "vad-earshot")]
    let mut gate = GateState::Idle;
    let mut loaded = false;
    loop {
        match inputs.try_recv() {
            Ok(Input::Enter) if loaded => {
                #[cfg(feature = "vad-earshot")]
                match gate {
                    GateState::Idle => stt.send(SttRequest::Arm),
                    GateState::Armed | GateState::Open => stt.send(SttRequest::Close),
                    GateState::Closing => {}
                }
                #[cfg(not(feature = "vad-earshot"))]
                if let Some(text) = last.clone() {
                    tts.send(TtsRequest::Speak {
                        text,
                        voice: args.voice.clone(),
                    });
                }
            }
            Ok(Input::Vocab(names)) => {
                #[cfg(feature = "stt-moshi-xn-boost")]
                stt.send(libvoicexn::stt::SttRequest::Vocabulary(names));
                #[cfg(not(feature = "stt-moshi-xn-boost"))]
                terminal.log(|| log::warn!("vocab: {names:?}: built without stt-moshi-xn-boost"));
            }
            Ok(Input::FileEnd) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Ok(Input::Enter) | Err(mpsc::TryRecvError::Empty) => {}
        }
        for event in stt.poll() {
            match event {
                SttEvent::Loaded { load_ms } => {
                    loaded = true;
                    if let Some(((mic, wav), done)) = file.take() {
                        std::thread::spawn(move || feed_file(mic, wav, done));
                    }
                    let doing = if cfg!(feature = "vad-earshot") {
                        "press Enter to talk"
                    } else {
                        "listening"
                    };
                    terminal.log(|| log::info!("stt: loaded in {load_ms} ms; {doing}"));
                }
                SttEvent::LoadFailed(e) => return Err(format!("stt: {e}")),
                SttEvent::Partial(text) => terminal.partial(&text),
                SttEvent::Utterance(text) => {
                    terminal.line(&format!("> {text}"));
                    tts.send(TtsRequest::Speak {
                        text: text.clone(),
                        voice: args.voice.clone(),
                    });
                    #[cfg(not(feature = "vad-earshot"))]
                    {
                        last = Some(text);
                    }
                }
                SttEvent::Speed { step_ms, dropped } => terminal
                    .log(|| log::info!("stt: step {step_ms:.1} ms of 80, dropped {dropped}")),
                #[cfg(feature = "vad-earshot")]
                SttEvent::Gate(state) => {
                    gate = state;
                    terminal.log(|| log::info!("stt: gate {state:?}"));
                }
                #[cfg(feature = "stt-moshi-xn-boost")]
                SttEvent::Vocabulary { phrases, sequences } => terminal.log(|| {
                    log::info!("stt: vocabulary {phrases} phrases ({sequences} sequences)")
                }),
            }
        }
        for event in tts.poll() {
            match event {
                TtsEvent::Loaded { voices, load_ms } => terminal
                    .log(|| log::info!("tts: loaded in {load_ms} ms, {} voices", voices.len())),
                TtsEvent::LoadFailed(e) => return Err(format!("tts: {e}")),
                TtsEvent::Finished {
                    audio_ms, synth_ms, ..
                } => terminal.log(|| log::info!("tts: {audio_ms} ms of audio in {synth_ms} ms")),
                TtsEvent::Failed { error, .. } => terminal.log(|| log::error!("tts: {error}")),
                TtsEvent::Started { .. }
                | TtsEvent::FirstAudio { .. }
                | TtsEvent::Stopped { .. } => {}
            }
        }
        std::thread::sleep(TICK);
    }
}

/// An interleaved WAV file as it is.
struct Wav {
    rate: u32,
    channels: u16,
    samples: Vec<f32>,
}

/// A 16-bit PCM or 32-bit float WAV file.
fn read_wav(path: &std::path::Path) -> Result<Wav, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut format, mut channels, mut rate, mut bits, mut data) = (0, 0, 0, 0, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => {
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let data = data.ok_or("no data chunk")?;
    let samples = match (format, bits) {
        (1, 16) => data
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        _ => return Err(format!("unsupported WAV format {format}, {bits} bits")),
    };
    if rate == 0 || channels == 0 {
        return Err("no fmt chunk".into());
    }
    Ok(Wav {
        rate,
        channels,
        samples,
    })
}

/// Pushes `wav` through `mic` in [`BLOCK`]s at real time, between [`LEAD`]
/// and [`TAIL`] of silence, as a device would, so that the first word is
/// heard, the last words come out and the utterance ends; then tells the
/// main loop to quit.
fn feed_file(mut mic: Mic, wav: Wav, done: mpsc::Sender<Input>) {
    let block = (wav.rate as usize * BLOCK.as_millis() as usize / 1000) * wav.channels as usize;
    let silence = |d: Duration| vec![0.0; block * (d.as_millis() / BLOCK.as_millis()) as usize];
    let (lead, tail) = (silence(LEAD), silence(TAIL));
    let blocks = lead.chunks(block);
    let blocks = blocks
        .chain(wav.samples.chunks(block))
        .chain(tail.chunks(block));
    let start = Instant::now();
    for (i, chunk) in blocks.enumerate() {
        mic.push(chunk);
        let next = start + BLOCK * (i as u32 + 1);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    let _ = done.send(Input::FileEnd);
}

/// The partial line on stdout, redrawn in place with `\r`, kept apart from
/// the log lines on stderr.
#[derive(Default)]
struct Terminal {
    partial: String,
}

impl Terminal {
    fn partial(&mut self, text: &str) {
        self.partial = text.to_string();
        self.redraw();
    }

    /// Ends the partial line with `text`.
    fn line(&mut self, text: &str) {
        self.partial.clear();
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "\r\x1b[K{text}");
        let _ = out.flush();
    }

    /// Clears the partial line for a log line, then draws it again.
    fn log(&mut self, log: impl FnOnce()) {
        if !self.partial.is_empty() {
            let mut out = std::io::stdout().lock();
            let _ = write!(out, "\r\x1b[K");
            let _ = out.flush();
        }
        log();
        self.redraw();
    }

    fn redraw(&self) {
        if self.partial.is_empty() {
            return;
        }
        let mut out = std::io::stdout().lock();
        let _ = write!(out, "\r\x1b[K\u{2026} {}", self.partial);
        let _ = out.flush();
    }
}
