//! A speech session for an app, behind `session`
//! (docs/design/20261029_01_SESSION_PLAN.md): both workers, with
//! `vad-earshot` the gate behind the STT worker, the speaker's ring and the [`Pump`] that
//! says when the app is speaking and listening. Moved from flankr's
//! `flankr-audio` 0.2.0, without its flankr types.
//!
//! The core opens no device. [`Session::spawn`] returns the two device
//! callbacks' ends: feed the input callback's samples to [`Mic::push`],
//! and fill the output callback's buffer with [`Playback::fill`]. A
//! libmorph app does that with two closures (README, "Using libvoicexn in
//! a libmorph app"); with `cpal`, [`Session::open_default`] opens the
//! default devices.
//!
//! [`Pump`]: pump::Pump

#[cfg(feature = "cpal")]
mod devices;
pub mod pump;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::capture::Mic;
use crate::config::{SttConfig, TtsConfig};
use crate::playback::{self, Playback};
use crate::stt::SttWorker;
use crate::tts::TtsWorker;
#[cfg(feature = "vad-earshot")]
use crate::vad::GateState;

/// The speaker's ring, in seconds of output.
const PLAYBACK_SECONDS: usize = 2;
/// The microphone's ring, in seconds of 24 kHz audio.
const CAPTURE_SECONDS: u32 = 4;

/// A device's rate and interleaved channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
}

/// What the session loads, and where it keeps the vocabulary.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Kyutai STT: the model folder, the quantization and the boost.
    pub stt: SttConfig,
    /// Phonon: the checkpoint folder and quantization.
    pub tts: TtsConfig,
    /// The Phonon voice for every `Speak`; `None` is the checkpoint's own.
    pub voice: Option<String>,
    /// `vocab.txt`. If it exists when the session starts, the STT boost
    /// reads it in place of the list [`SttConfig::for_platform`] found
    /// (`VOICEXN_VOCAB` included); [`Action::Vocabulary`] writes it for the
    /// next start. `None` reads and writes nothing.
    pub vocab_file: Option<PathBuf>,
    /// Mute the microphone while the speaker plays, so STT does not hear
    /// Phonon. On by default; off for input that has no echo, such as a
    /// file.
    pub echo_mute: bool,
}

impl SessionConfig {
    /// libvoicexn's layout under `models`: `phonon-31m-en` and
    /// `stt-1b-en_fr`, with the platform's defaults and the `VOICEXN_*`
    /// overrides, and no vocabulary file.
    pub fn new(models: impl AsRef<Path>) -> Self {
        let models = models.as_ref();
        Self {
            stt: SttConfig::for_platform(models.join("stt-1b-en_fr")),
            tts: TtsConfig::for_platform(models.join("phonon-31m-en")),
            voice: None,
            vocab_file: None,
            echo_mute: true,
        }
    }
}

/// What an app asks of the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Say `text` in the session's voice.
    Speak(String),
    /// Stop Phonon and drop what is queued for the speaker.
    Stop,
    /// With `vad-earshot`, arm the gate: tap to talk.
    Arm,
    /// With `vad-earshot`, close the gate while it is listening.
    Close,
    /// Replace the boosted phrases: on the running model from the next
    /// utterance with `stt-moshi-xn-boost`, and in `vocab_file` for the
    /// next start.
    Vocabulary(Vec<String>),
    /// Play the cue (a short chime) over whatever the speaker plays.
    Cue,
}

/// What the session reports. `Utterance`, `Listening`, `Voice` and
/// `Speaking` are the app's turn state; the rest are for it to show or log.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// One ended utterance.
    Utterance(String),
    /// The utterance so far.
    Partial(String),
    /// With `vad-earshot`: from the tap (`Armed`) until the utterance has
    /// ended (`Idle`).
    Listening(bool),
    /// With `vad-earshot`: the gate is open on speech.
    Voice(bool),
    /// From the first `Speak` until nothing is queued, nothing is being
    /// made and the speaker's ring has run dry.
    Speaking(bool),
    /// The earshot gate changed.
    #[cfg(feature = "vad-earshot")]
    Gate(GateState),
    /// The STT model is loaded; [`Action::Arm`] works from here.
    SttLoaded { load_ms: u32 },
    /// The STT model did not load. Nothing will be heard.
    SttLoadFailed(String),
    /// Phonon is loaded, with these voices (the checkpoint's `voices/`, in
    /// Phonon's order); any of them can go to [`Session::set_voice`].
    TtsLoaded {
        load_ms: u32,
        voices: usize,
        names: Vec<String>,
    },
    /// Phonon did not load. Nothing will be said.
    TtsLoadFailed(String),
    /// The STT model's mean step over the last 25 frames (budget 80 ms),
    /// and the samples dropped since it loaded.
    Speed { step_ms: f32, dropped: u64 },
    /// One `Speak` was made.
    Spoken { audio_ms: u32, synth_ms: u32 },
    /// One `Speak` failed.
    SpeakFailed(String),
    /// A phrase list took effect on the running model.
    #[cfg(feature = "stt-moshi-xn-boost")]
    Vocabulary { phrases: usize, sequences: usize },
    /// `vocab_file` was written, with `phrases` lines.
    VocabularySaved { path: PathBuf, phrases: usize },
    /// `vocab_file` could not be written.
    VocabularyFailed(String),
}

/// The workers, and the session's half of the device callbacks. With
/// `cpal` from [`open_default`](Self::open_default) it also holds the
/// streams, which are not `Send` on every platform.
pub struct Session {
    tts: TtsWorker,
    stt: SttWorker,
    pump: pump::Pump,
    voice: Option<String>,
    vocab_file: Option<PathBuf>,
    /// Moved on every `Stop`; [`Playback`] drains the ring when it moves.
    flush: Arc<AtomicU32>,
    /// Moved on every `Cue`; [`Playback`] starts the cue when it moves.
    cues: Arc<AtomicU32>,
    /// Set while the speaker's last buffer played speech.
    playing: Arc<AtomicBool>,
    /// Events made by [`act`](Self::act), given by the next poll.
    pending: Vec<SessionEvent>,
    #[cfg(feature = "cpal")]
    _streams: Option<devices::Streams>,
}

impl Session {
    /// Starts the workers for a microphone in `input` and a speaker in
    /// `output`, and returns the two device callbacks' ends. The models
    /// load on the workers' threads; [`poll`](Self::poll) says when.
    pub fn spawn(config: SessionConfig, input: Format, output: Format) -> (Self, Mic, Playback) {
        let capacity = output.rate as usize * PLAYBACK_SECONDS;
        let (producer, consumer) = playback::ring(capacity, output.channels, output.rate);
        let playing = consumer.speaking();
        // Muted while the speaker plays, so STT does not hear Phonon.
        let mute = if config.echo_mute {
            playing.clone()
        } else {
            Arc::new(AtomicBool::new(false))
        };
        let (mic, capture) = Mic::new(input.rate, input.channels, mute, CAPTURE_SECONDS);
        let flush = Arc::new(AtomicU32::new(0));
        let cues = Arc::new(AtomicU32::new(0));
        let playback = Playback::new(consumer, flush.clone())
            .with_cue(playback::chime(output.rate), cues.clone());

        // A vocabulary file the app wrote last time wins over the list
        // `SttConfig::for_platform` found.
        #[cfg_attr(not(feature = "stt-moshi-xn-boost"), allow(unused_mut))]
        let mut stt_config = config.stt;
        #[cfg(feature = "stt-moshi-xn-boost")]
        if let Some(path) = config.vocab_file.as_ref().filter(|p| p.is_file()) {
            stt_config.vocab = Some(path.clone());
        }
        log::info!(
            "session: stt {} ({:?}); tts {}",
            stt_config.model_dir.display(),
            stt_config.quant,
            config.tts.model_dir.display()
        );
        let tts = TtsWorker::spawn(config.tts, Some(producer));
        let stt = SttWorker::spawn(stt_config, capture);
        let session = Self {
            tts,
            stt,
            pump: pump::Pump::new(),
            voice: config.voice,
            vocab_file: config.vocab_file,
            flush,
            cues,
            playing,
            pending: Vec::new(),
            #[cfg(feature = "cpal")]
            _streams: None,
        };
        (session, mic, playback)
    }

    /// Opens the default input and output devices with cpal and starts the
    /// session on them; the session holds the streams.
    #[cfg(feature = "cpal")]
    pub fn open_default(config: SessionConfig) -> Result<Self, String> {
        let (input, output) = (devices::Device::input()?, devices::Device::output()?);
        let (mut session, mic, playback) = Self::spawn(config, input.format(), output.format());
        session._streams = Some(devices::Streams {
            _output: output.play(playback)?,
            _input: Some(input.capture(mic)?),
        });
        Ok(session)
    }

    /// Opens only the default output device with cpal, for a microphone the
    /// caller feeds through the returned [`Mic`] in `input`'s format: a
    /// machine with no input device, or audio from a file.
    #[cfg(feature = "cpal")]
    pub fn open_default_output(
        config: SessionConfig,
        input: Format,
    ) -> Result<(Self, Mic), String> {
        let output = devices::Device::output()?;
        let (mut session, mic, playback) = Self::spawn(config, input, output.format());
        session._streams = Some(devices::Streams {
            _output: output.play(playback)?,
            _input: None,
        });
        Ok((session, mic))
    }

    /// Carries out one [`Action`].
    pub fn act(&mut self, action: Action) {
        if let Some(e) = self.pump.on_action(&action) {
            self.pending.push(e);
        }
        if let Some(req) = pump::tts_request(&action, self.voice.as_deref()) {
            self.tts.send(req);
        }
        #[cfg(any(feature = "vad-earshot", feature = "stt-moshi-xn-boost"))]
        if let Some(req) = pump::stt_request(&action) {
            self.stt.send(req);
        }
        match action {
            Action::Stop => {
                self.flush.fetch_add(1, Ordering::Release);
            }
            Action::Vocabulary(words) => {
                if let Some(e) = self.save_vocabulary(&words) {
                    self.pending.push(e);
                }
            }
            Action::Cue => {
                self.cues.fetch_add(1, Ordering::Release);
            }
            Action::Speak(_) | Action::Arm | Action::Close => {}
        }
    }

    /// What has happened since the last poll, without waiting. Call it
    /// often (every 20 ms or so): `Speaking(false)` is noticed here.
    pub fn poll(&mut self) -> Vec<SessionEvent> {
        let mut out = std::mem::take(&mut self.pending);
        for event in self.stt.poll() {
            out.extend(self.pump.on_stt(event));
        }
        for event in self.tts.poll() {
            out.extend(self.pump.on_tts(event));
        }
        out.extend(self.pump.tick(self.playing.load(Ordering::Relaxed)));
        out
    }

    /// The Phonon voice every later `Speak` is said in; `None` is the
    /// checkpoint's own. What is already being said keeps its voice. A name
    /// Phonon does not have makes each `Speak` fail with
    /// [`SessionEvent::SpeakFailed`].
    pub fn set_voice(&mut self, voice: Option<String>) {
        if voice != self.voice {
            log::info!("session: voice {}", voice.as_deref().unwrap_or("(default)"));
            self.voice = voice;
        }
    }

    /// The voice `Speak` is said in.
    pub fn voice(&self) -> Option<&str> {
        self.voice.as_deref()
    }

    /// Where `Vocabulary` is written, if anywhere.
    pub fn vocab_file(&self) -> Option<&Path> {
        self.vocab_file.as_deref()
    }

    fn save_vocabulary(&self, words: &[String]) -> Option<SessionEvent> {
        let path = self.vocab_file.as_ref()?;
        Some(match pump::write_vocab(path, words) {
            Ok(phrases) => SessionEvent::VocabularySaved {
                path: path.clone(),
                phrases,
            },
            Err(e) => SessionEvent::VocabularyFailed(format!("{}: {e}", path.display())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::TtsRequest;

    /// A session on models that are not there: the workers report the
    /// failure and stop, and the session still takes requests.
    fn session(vocab: bool) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = SessionConfig::new(dir.path());
        config.vocab_file = vocab.then(|| dir.path().join("vocab.txt"));
        config.voice = Some("alba".into());
        let format = Format {
            rate: 24_000,
            channels: 1,
        };
        let (session, _mic, _playback) = Session::spawn(config, format, format);
        (dir, session)
    }

    #[test]
    fn the_voice_is_set_while_the_session_runs() {
        let (_dir, mut s) = session(false);
        assert_eq!(s.voice(), Some("alba"), "from the config");
        s.set_voice(Some("cosette".into()));
        assert_eq!(s.voice(), Some("cosette"));
        let speak = pump::tts_request(&Action::Speak("This is cosette.".into()), s.voice());
        assert!(matches!(
            speak,
            Some(TtsRequest::Speak { voice: Some(ref v), .. }) if v == "cosette"
        ));
        s.act(Action::Speak("This is cosette.".into()));
        s.set_voice(None);
        assert_eq!(s.voice(), None, "back to Phonon's own");
    }

    #[test]
    fn vocabulary_is_written_only_with_a_file() {
        let (dir, mut s) = session(true);
        s.act(Action::Vocabulary(vec![
            "libmorph".into(),
            "roundtwo".into(),
        ]));
        let events = s.poll();
        assert!(
            events.contains(&SessionEvent::VocabularySaved {
                path: dir.path().join("vocab.txt"),
                phrases: 2
            }),
            "{events:?}"
        );
        let (_dir, mut s) = session(false);
        s.act(Action::Vocabulary(vec!["libmorph".into()]));
        assert!(!s.poll().iter().any(|e| matches!(
            e,
            SessionEvent::VocabularySaved { .. } | SessionEvent::VocabularyFailed(_)
        )));
    }

    #[test]
    fn a_speak_is_speaking_at_once() {
        let (_dir, mut s) = session(false);
        s.act(Action::Speak("Hello.".into()));
        assert!(s.poll().contains(&SessionEvent::Speaking(true)));
    }
}
