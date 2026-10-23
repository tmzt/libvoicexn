//! libvoicexn in a terminal (docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md,
//! decision 8): the default microphone into STT, each utterance printed as
//! `> text` and said back by Phonon on the default output. No window and no
//! libmorph: `cpal` opens the devices, `Mic::push` and `Consumer::fill` are
//! its callbacks.
//!
//! ```text
//! cargo run --release -p voicexn-tty [--features vad-earshot] -- \
//!     [--quant q4k|q4_0|q8] [--voice <name>] [--vocab <file>]
//! ```
//!
//! Enter does what a tap does in voicexn: with `vad-earshot` it arms the
//! gate, or closes it while it is listening; without, it repeats the last
//! utterance. End of input (Ctrl-D) quits.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

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

struct Args {
    quant: Option<SttQuant>,
    voice: Option<String>,
    vocab: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut parsed = Args {
        quant: None,
        voice: None,
        vocab: None,
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
            _ => {
                return Err(format!(
                    "unknown argument {arg:?}\nusage: voicexn-tty [--quant q4k|q4_0|q8] [--voice <name>] [--vocab <file>]"
                ));
            }
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

fn run(args: Args) -> Result<(), String> {
    let host = cpal::default_host();
    let output = host.default_output_device().ok_or("no output device")?;
    let input = host.default_input_device().ok_or("no input device")?;
    let out_config = output.default_output_config().map_err(|e| e.to_string())?;
    let in_config = input.default_input_config().map_err(|e| e.to_string())?;
    let (out_rate, out_channels) = (out_config.sample_rate(), out_config.channels());
    let (in_rate, in_channels) = (in_config.sample_rate(), in_config.channels());
    log::info!("playback: {out_rate} Hz, {out_channels} channels");
    log::info!("capture: {in_rate} Hz, {in_channels} channels");

    // Two seconds of output, and a microphone muted while it plays.
    let (producer, mut consumer) = playback::ring(out_rate as usize * 2, out_channels, out_rate);
    let (mut mic, capture) = Mic::new(in_rate, in_channels, consumer.speaking(), 4);
    let out_stream = output
        .build_output_stream::<f32, _, _>(
            out_config.config(),
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| consumer.fill(out),
            |e| log::error!("playback: {e}"),
            None,
        )
        .map_err(|e| format!("playback: {e}"))?;
    let in_stream = input
        .build_input_stream::<f32, _, _>(
            in_config.config(),
            move |samples: &[f32], _: &cpal::InputCallbackInfo| mic.push(samples),
            |e| log::error!("capture: {e}"),
            None,
        )
        .map_err(|e| format!("capture: {e}"))?;
    out_stream.play().map_err(|e| format!("playback: {e}"))?;
    in_stream.play().map_err(|e| format!("capture: {e}"))?;

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

    // Enter and end of input, from a thread of their own.
    let (keys, enter) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if line.is_err() || keys.send(()).is_err() {
                return;
            }
        }
    });

    let mut terminal = Terminal::default();
    #[cfg(not(feature = "vad-earshot"))]
    let mut last = None::<String>;
    #[cfg(feature = "vad-earshot")]
    let mut gate = GateState::Idle;
    let mut loaded = false;
    loop {
        match enter.try_recv() {
            Ok(()) if loaded => {
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
            Ok(()) | Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
        }
        for event in stt.poll() {
            match event {
                SttEvent::Loaded { load_ms } => {
                    loaded = true;
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
