//! The desktop entry: the app in libmorph's emulated phone, or, with
//! `--say "<text>"`, one utterance written to `out/tts/<unix ms>.wav`.
//!
//! ```text
//! cargo run --release -- --say "Hello from voicexn." [--model <dir>] [--voice <name>]
//! cargo run --release --features stt-moshi-xn -- --transcribe <wav> [--model <dir>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use voicexn::tts::{TtsEvent, TtsRequest, TtsWorker};

const MODEL_DIR: &str = "models/phonon-31m-en";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--say") {
        return say(&args[1..]);
    }
    #[cfg(feature = "stt-moshi-xn")]
    if args.first().map(String::as_str) == Some("--transcribe") {
        return transcribe(&args[1..]);
    }
    match libmorph::run::<voicexn::App>() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn say(args: &[String]) -> ExitCode {
    let mut text = None;
    let mut model = PathBuf::from(MODEL_DIR);
    let mut voice = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next().map_or(model, PathBuf::from),
            "--voice" => voice = args.next().cloned(),
            _ => text = Some(arg.clone()),
        }
    }
    let Some(text) = text else {
        log::error!("usage: voicexn --say \"<text>\" [--model <dir>] [--voice <name>]");
        return ExitCode::FAILURE;
    };

    let worker = TtsWorker::spawn(model, None, Some(PathBuf::from("out/tts")));
    worker.send(TtsRequest::Speak { text, voice });
    while let Some(event) = worker.wait() {
        match event {
            TtsEvent::Loaded { voices, load_ms } => {
                log::info!("tts: loaded in {load_ms} ms, {} voices", voices.len());
            }
            TtsEvent::FirstAudio { ms, .. } => log::info!("tts: first audio after {ms} ms"),
            TtsEvent::Finished {
                audio_ms, synth_ms, ..
            } => {
                log::info!(
                    "tts: {audio_ms} ms of audio in {synth_ms} ms, RTF {:.3}",
                    f64::from(synth_ms) / f64::from(audio_ms.max(1))
                );
                return ExitCode::SUCCESS;
            }
            TtsEvent::LoadFailed(error) | TtsEvent::Failed { error, .. } => {
                log::error!("tts: {error}");
                return ExitCode::FAILURE;
            }
            TtsEvent::Started { .. } | TtsEvent::Stopped { .. } => {}
        }
    }
    ExitCode::FAILURE
}

/// Transcribes a WAV file (plan 20261022_02, step 2).
#[cfg(feature = "stt-moshi-xn")]
fn transcribe(args: &[String]) -> ExitCode {
    let mut wav = None;
    let mut model = PathBuf::from("models/stt-1b-en_fr");
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next().map_or(model, PathBuf::from),
            _ => wav = Some(PathBuf::from(arg)),
        }
    }
    let Some(wav) = wav else {
        log::error!("usage: voicexn --transcribe <wav> [--model <dir>]");
        return ExitCode::FAILURE;
    };
    match voicexn::stt::transcribe_file(&model, &wav) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            log::error!("stt: {e}");
            ExitCode::FAILURE
        }
    }
}
