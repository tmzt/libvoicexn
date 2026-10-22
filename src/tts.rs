//! The `tts` thread: it loads Phonon once and speaks the requests it is
//! sent, one at a time (plan section 2, decision 3). `ptts` calls block and
//! must come from one thread, so nothing else touches the model.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ptts::checkpoint::{Checkpoint, ResolveOptions};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};

use crate::playback::Producer;

pub enum TtsRequest {
    Speak { text: String, voice: Option<String> },
    Stop,
}

#[derive(Debug)]
pub enum TtsEvent {
    Loaded {
        voices: Vec<String>,
        load_ms: u32,
    },
    LoadFailed(String),
    Started {
        id: u64,
    },
    /// From the request to its first chunk of audio.
    FirstAudio {
        id: u64,
        ms: u32,
    },
    /// The real-time factor is `synth_ms / audio_ms`. `synth_ms` leaves out
    /// time spent waiting for room in the playback ring.
    Finished {
        id: u64,
        audio_ms: u32,
        synth_ms: u32,
    },
    Stopped {
        id: u64,
    },
    Failed {
        id: u64,
        error: String,
    },
}

pub struct TtsWorker {
    tx: Sender<TtsRequest>,
    rx: Receiver<TtsEvent>,
}

impl TtsWorker {
    /// Starts the thread, which loads the checkpoint in `model_dir`. Audio
    /// goes to `playback` as it is made. With `record`, each utterance is
    /// also written to `<record>/<unix ms>.wav`.
    pub fn spawn(model_dir: PathBuf, playback: Option<Producer>, record: Option<PathBuf>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        thread::Builder::new()
            .name("tts".into())
            .spawn(move || {
                let start = Instant::now();
                let tts = match load(&model_dir) {
                    Ok(tts) => tts,
                    Err(e) => {
                        let _ = events.send(TtsEvent::LoadFailed(format!(
                            "{}: {e}",
                            model_dir.display()
                        )));
                        return;
                    }
                };
                let _ = events.send(TtsEvent::Loaded {
                    voices: tts.voices(),
                    load_ms: ms(start),
                });
                Worker {
                    tts,
                    playback,
                    record,
                    requests,
                    pending: VecDeque::new(),
                    events,
                }
                .run();
            })
            .expect("spawning the tts thread");
        Self { tx, rx }
    }

    pub fn send(&self, req: TtsRequest) {
        // The thread only exits once this sender is dropped.
        let _ = self.tx.send(req);
    }

    pub fn poll(&self) -> impl Iterator<Item = TtsEvent> + '_ {
        self.rx.try_iter()
    }

    /// Waits for the next event; `None` once the thread has gone.
    pub fn wait(&self) -> Option<TtsEvent> {
        self.rx.recv().ok()
    }
}

fn load(model_dir: &Path) -> ptts::Result<Synth> {
    let checkpoint = Checkpoint::resolve(
        model_dir,
        ResolveOptions {
            quant: Quant::Q80,
            weights: None,
        },
    )?;
    let mut tts = checkpoint
        .builder(Lang::En)
        .device(DeviceKind::Cpu)
        .build()?;
    checkpoint.register_voices(&mut tts);
    Ok(tts)
}

/// The `tts` thread's state once the model is loaded.
struct Worker {
    tts: Synth,
    playback: Option<Producer>,
    record: Option<PathBuf>,
    requests: Receiver<TtsRequest>,
    /// Requests that arrived while an utterance was playing.
    pending: VecDeque<TtsRequest>,
    events: Sender<TtsEvent>,
}

impl Worker {
    fn run(mut self) {
        let mut next_id = 0;
        while let Some(req) = self
            .pending
            .pop_front()
            .or_else(|| self.requests.recv().ok())
        {
            let TtsRequest::Speak { text, voice } = req else {
                continue; // Stop with nothing playing
            };
            next_id += 1;
            let last = self.speak(next_id, &text, voice);
            if let (TtsEvent::Stopped { .. }, Some(playback)) = (&last, &mut self.playback) {
                playback.flush();
            }
            let _ = self.events.send(last);
        }
    }

    /// Takes the requests that have arrived. `true` on `Stop`, which also
    /// drops whatever was queued; a `Speak` waits its turn in `pending`.
    fn stop_requested(requests: &Receiver<TtsRequest>, pending: &mut VecDeque<TtsRequest>) -> bool {
        loop {
            match requests.try_recv() {
                Ok(TtsRequest::Stop) => {
                    pending.clear();
                    return true;
                }
                Ok(speak) => pending.push_back(speak),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Speaks one request and returns its last event.
    fn speak(&mut self, id: u64, text: &str, voice: Option<String>) -> TtsEvent {
        let start = Instant::now();
        let _ = self.events.send(TtsEvent::Started { id });
        let mut options = SpeechOptions::default();
        if let Some(voice) = voice {
            options = options.voice(voice);
        }
        let stream = match self.tts.stream_with(text, &options) {
            Ok(stream) => stream,
            Err(e) => {
                return TtsEvent::Failed {
                    id,
                    error: e.to_string(),
                };
            }
        };
        if let Some(playback) = &mut self.playback {
            playback.reset();
        }
        let mut samples = 0;
        let mut pcm = Vec::new();
        let mut blocked = Duration::ZERO;
        for chunk in stream {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    return TtsEvent::Failed {
                        id,
                        error: e.to_string(),
                    };
                }
            };
            if samples == 0 {
                let _ = self.events.send(TtsEvent::FirstAudio { id, ms: ms(start) });
            }
            samples += chunk.len();
            if self.record.is_some() {
                pcm.extend_from_slice(&chunk);
            }
            let (requests, pending) = (&self.requests, &mut self.pending);
            if let Some(playback) = &mut self.playback {
                let wait = Instant::now();
                let played = playback.push(&chunk, || Self::stop_requested(requests, pending));
                blocked += wait.elapsed();
                if !played {
                    return TtsEvent::Stopped { id };
                }
            }
            if Self::stop_requested(requests, pending) {
                return TtsEvent::Stopped { id };
            }
        }
        let synth_ms = (start.elapsed() - blocked).as_millis() as u32;
        let rate = self.tts.sample_rate();
        if let Some(dir) = &self.record {
            let path = dir.join(format!("{}.wav", unix_ms()));
            match std::fs::create_dir_all(dir)
                .map_err(ptts::Error::from)
                .and_then(|()| ptts::wav::write_wav_file(&path, &pcm, rate))
            {
                Ok(()) => log::info!("tts: wrote {}", path.display()),
                Err(e) => log::warn!("tts: writing {}: {e}", path.display()),
            }
        }
        TtsEvent::Finished {
            id,
            audio_ms: (samples as u64 * 1000 / u64::from(rate)) as u32,
            synth_ms,
        }
    }
}

fn ms(since: Instant) -> u32 {
    since.elapsed().as_millis() as u32
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}
