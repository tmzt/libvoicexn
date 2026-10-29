//! The mapping between the workers and a [`Session`](super::Session),
//! kept pure: no threads, devices or models, so it is tested on its own
//! (docs/design/20261029_01_SESSION_PLAN.md). Moved from flankr's
//! `flankr-audio` 0.2.0, without its flankr types.
//!
//! - An [`Action`] becomes requests for the workers ([`tts_request`],
//!   [`stt_request`]).
//! - [`SttEvent`]s and [`TtsEvent`]s become [`SessionEvent`]s; the
//!   [`Pump`] adds `Listening`, `Voice` and `Speaking`, each only when it
//!   changes.

use crate::stt::SttEvent;
#[cfg(any(feature = "vad-earshot", feature = "stt-moshi-xn-boost"))]
use crate::stt::SttRequest;
use crate::tts::{TtsEvent, TtsRequest};
#[cfg(feature = "vad-earshot")]
use crate::vad::GateState;

use super::{Action, SessionEvent};

/// The TTS worker's request for `action`, spoken in `voice`.
pub fn tts_request(action: &Action, voice: Option<&str>) -> Option<TtsRequest> {
    match action {
        Action::Speak(text) => Some(TtsRequest::Speak {
            text: text.clone(),
            voice: voice.map(str::to_string),
        }),
        Action::Stop => Some(TtsRequest::Stop),
        Action::Arm | Action::Close | Action::Vocabulary(_) | Action::Cue => None,
    }
}

/// The STT worker's request for `action`: with `vad-earshot`, `Arm` and
/// `Close`, and with `stt-moshi-xn-boost`, `Vocabulary` for the running
/// model.
#[cfg(any(feature = "vad-earshot", feature = "stt-moshi-xn-boost"))]
pub fn stt_request(action: &Action) -> Option<SttRequest> {
    match action {
        #[cfg(feature = "vad-earshot")]
        Action::Arm => Some(SttRequest::Arm),
        #[cfg(feature = "vad-earshot")]
        Action::Close => Some(SttRequest::Close),
        #[cfg(feature = "stt-moshi-xn-boost")]
        Action::Vocabulary(words) => Some(SttRequest::Vocabulary(words.clone())),
        _ => None,
    }
}

/// Whether the gate counts as listening for `SessionEvent::Listening`: from the
/// tap (`Armed`) until the utterance has ended (`Idle`), as
/// [`Gate::listening`](crate::vad::Gate::listening) has it.
#[cfg(feature = "vad-earshot")]
pub fn gate_listening(state: GateState) -> bool {
    state != GateState::Idle
}

/// The state behind `Speaking`, `Listening` and `Voice`, so each
/// is given only when it changes.
///
/// Speaking starts when a `Say` is sent to the TTS worker and lasts until
/// the worker has nothing queued or playing *and* the speaker's ring has
/// run dry, so a notice is not released while the end of a reply is still
/// in the ring.
#[derive(Debug, Default)]
pub struct Pump {
    /// `Speak`s sent and not yet started.
    queued: usize,
    /// An utterance between `Started` and its last event.
    active: bool,
    /// The last `Speaking` given.
    speaking: bool,
    /// The last `Listening` given.
    #[cfg(feature = "vad-earshot")]
    listening: bool,
    /// The last `Voice` given.
    #[cfg(feature = "vad-earshot")]
    voice: bool,
}

impl Pump {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes `action` going to the workers. `Speak` gives `Speaking(true)`
    /// at once if it is not in effect.
    pub fn on_action(&mut self, action: &Action) -> Option<SessionEvent> {
        match action {
            Action::Speak(_) => {
                self.queued += 1;
                self.set_speaking(true)
            }
            // The worker drops what it had queued; the utterance playing
            // ends with `Stopped`.
            Action::Stop => {
                self.queued = 0;
                None
            }
            // The cue is not speech: it holds no notice back.
            Action::Arm | Action::Close | Action::Vocabulary(_) | Action::Cue => None,
        }
    }

    /// Maps one STT event.
    pub fn on_stt(&mut self, event: SttEvent) -> Vec<SessionEvent> {
        match event {
            SttEvent::Loaded { load_ms } => vec![SessionEvent::SttLoaded { load_ms }],
            SttEvent::LoadFailed(e) => vec![SessionEvent::SttLoadFailed(e)],
            SttEvent::Partial(text) => vec![SessionEvent::Partial(text)],
            SttEvent::Utterance(text) => vec![SessionEvent::Utterance(text)],
            SttEvent::Speed { step_ms, dropped } => vec![SessionEvent::Speed { step_ms, dropped }],
            #[cfg(feature = "stt-moshi-xn-boost")]
            SttEvent::Vocabulary { phrases, sequences } => {
                vec![SessionEvent::Vocabulary { phrases, sequences }]
            }
            #[cfg(feature = "vad-earshot")]
            SttEvent::Gate(state) => {
                let mut out = vec![SessionEvent::Gate(state)];
                let listening = gate_listening(state);
                if listening != self.listening {
                    self.listening = listening;
                    out.push(SessionEvent::Listening(listening));
                }
                // Voice is the gate open on speech; armed and waiting is not.
                let voice = state == GateState::Open;
                if voice != self.voice {
                    self.voice = voice;
                    out.push(SessionEvent::Voice(voice));
                }
                out
            }
        }
    }

    /// Maps one TTS event. `Speaking(false)` waits for [`tick`](Self::tick).
    pub fn on_tts(&mut self, event: TtsEvent) -> Option<SessionEvent> {
        match event {
            TtsEvent::Loaded { voices, load_ms } => Some(SessionEvent::TtsLoaded {
                load_ms,
                voices: voices.len(),
                names: voices,
            }),
            TtsEvent::LoadFailed(e) => {
                self.queued = 0;
                self.active = false;
                Some(SessionEvent::TtsLoadFailed(e))
            }
            TtsEvent::Started { .. } => {
                self.queued = self.queued.saturating_sub(1);
                self.active = true;
                // A `Speak` sent before a `Stop` can still start.
                self.set_speaking(true)
            }
            TtsEvent::FirstAudio { .. } => None,
            TtsEvent::Finished {
                audio_ms, synth_ms, ..
            } => {
                self.active = false;
                Some(SessionEvent::Spoken { audio_ms, synth_ms })
            }
            TtsEvent::Stopped { .. } => {
                self.active = false;
                None
            }
            TtsEvent::Failed { error, .. } => {
                self.active = false;
                Some(SessionEvent::SpeakFailed(error))
            }
        }
    }

    /// After the events of one poll: `Speaking(false)` once nothing is
    /// queued, nothing is being made and the ring is not `playing`.
    pub fn tick(&mut self, playing: bool) -> Option<SessionEvent> {
        let speaking = self.queued > 0 || self.active || playing;
        if !speaking {
            return self.set_speaking(false);
        }
        None
    }

    fn set_speaking(&mut self, speaking: bool) -> Option<SessionEvent> {
        if speaking == self.speaking {
            return None;
        }
        self.speaking = speaking;
        Some(SessionEvent::Speaking(speaking))
    }
}

/// `vocab.txt` for `words`: a comment, then one phrase per line, in order,
/// without repeats. Line breaks inside a phrase become spaces; empty
/// phrases, and phrases that would read as comments, are left out.
pub fn vocab_text(words: &[String]) -> (String, usize) {
    let mut text = String::from(
        "# The vocabulary, written by libvoicexn's session.\n# libvoicexn reads it when the STT model loads.\n",
    );
    let mut seen = std::collections::HashSet::new();
    for word in words {
        let phrase = word
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if phrase.is_empty() || phrase.starts_with('#') || !seen.insert(phrase.clone()) {
            continue;
        }
        text.push_str(&phrase);
        text.push('\n');
    }
    (text, seen.len())
}

/// Writes `vocab.txt` for `words` at `path`, through a temporary file and a
/// rename, creating its folder. Returns the phrases written; a file that
/// already says the same is left alone.
pub fn write_vocab(path: &std::path::Path, words: &[String]) -> std::io::Result<usize> {
    let (text, phrases) = vocab_text(words);
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(phrases);
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(phrases)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The state changes among `events`: utterances, listening, voice
    /// and speaking.
    fn inputs(events: &[SessionEvent]) -> Vec<SessionEvent> {
        events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SessionEvent::Utterance(_)
                        | SessionEvent::Listening(_)
                        | SessionEvent::Voice(_)
                        | SessionEvent::Speaking(_)
                )
            })
            .cloned()
            .collect()
    }

    #[test]
    fn actions_become_worker_requests() {
        let speak = tts_request(&Action::Speak("Hi.".into()), Some("alba"));
        assert!(matches!(
            speak,
            Some(TtsRequest::Speak { ref text, voice: Some(ref v) }) if text == "Hi." && v == "alba"
        ));
        assert!(matches!(
            tts_request(&Action::Stop, None),
            Some(TtsRequest::Stop)
        ));
        assert!(tts_request(&Action::Arm, None).is_none());
        assert!(tts_request(&Action::Close, None).is_none());
        #[cfg(feature = "vad-earshot")]
        {
            assert!(matches!(stt_request(&Action::Arm), Some(SttRequest::Arm)));
            assert!(matches!(
                stt_request(&Action::Close),
                Some(SttRequest::Close)
            ));
        }
        #[cfg(any(feature = "vad-earshot", feature = "stt-moshi-xn-boost"))]
        assert!(stt_request(&Action::Stop).is_none());
        // The running model takes the list only with the boost built in.
        #[cfg(feature = "stt-moshi-xn-boost")]
        assert!(matches!(
            stt_request(&Action::Vocabulary(vec!["claude".into()])),
            Some(SttRequest::Vocabulary(ref w)) if w == &["claude".to_string()]
        ));
        #[cfg(all(feature = "vad-earshot", not(feature = "stt-moshi-xn-boost")))]
        assert!(stt_request(&Action::Vocabulary(vec![])).is_none());
        assert!(tts_request(&Action::Cue, None).is_none());
        #[cfg(any(feature = "vad-earshot", feature = "stt-moshi-xn-boost"))]
        assert!(stt_request(&Action::Cue).is_none());
    }

    #[test]
    fn an_utterance_is_an_input() {
        let mut pump = Pump::new();
        let out = pump.on_stt(SttEvent::Utterance("send continue to claude".into()));
        assert_eq!(
            inputs(&out),
            vec![SessionEvent::Utterance("send continue to claude".into())]
        );
        assert_eq!(
            pump.on_stt(SttEvent::Partial("send".into())),
            vec![SessionEvent::Partial("send".into())]
        );
    }

    #[cfg(feature = "vad-earshot")]
    #[test]
    fn listening_follows_the_gate_from_the_tap_to_idle() {
        let mut pump = Pump::new();
        let armed = pump.on_stt(SttEvent::Gate(GateState::Armed));
        assert_eq!(armed[0], SessionEvent::Gate(GateState::Armed));
        assert_eq!(inputs(&armed), vec![SessionEvent::Listening(true)]);
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Open))),
            vec![SessionEvent::Voice(true)]
        );
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Closing))),
            vec![SessionEvent::Voice(false)]
        );
        let idle = pump.on_stt(SttEvent::Gate(GateState::Idle));
        assert_eq!(inputs(&idle), vec![SessionEvent::Listening(false)]);
        // An arm that times out is still a change, and back.
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Armed))),
            vec![SessionEvent::Listening(true)]
        );
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Idle))),
            vec![SessionEvent::Listening(false)]
        );
    }

    #[test]
    fn speaking_lasts_until_the_ring_runs_dry() {
        let mut pump = Pump::new();
        let say = Action::Speak("Hello.".into());
        assert_eq!(pump.on_action(&say), Some(SessionEvent::Speaking(true)));
        // Not started yet: still speaking.
        assert_eq!(pump.tick(false), None);
        assert_eq!(pump.on_tts(TtsEvent::Started { id: 1 }), None);
        assert_eq!(pump.on_tts(TtsEvent::FirstAudio { id: 1, ms: 40 }), None);
        assert_eq!(
            pump.on_tts(TtsEvent::Finished {
                id: 1,
                audio_ms: 900,
                synth_ms: 120
            }),
            Some(SessionEvent::Spoken {
                audio_ms: 900,
                synth_ms: 120
            })
        );
        // The end of the reply is still in the ring.
        assert_eq!(pump.tick(true), None);
        assert_eq!(pump.tick(false), Some(SessionEvent::Speaking(false)));
        assert_eq!(pump.tick(false), None);
    }

    #[test]
    fn two_says_in_a_row_are_one_stretch_of_speaking() {
        let mut pump = Pump::new();
        assert!(pump.on_action(&Action::Speak("One.".into())).is_some());
        assert_eq!(pump.on_action(&Action::Speak("Two.".into())), None);
        pump.on_tts(TtsEvent::Started { id: 1 });
        pump.on_tts(TtsEvent::Finished {
            id: 1,
            audio_ms: 1,
            synth_ms: 1,
        });
        // "Two." is queued: the gap between them is not the end.
        assert_eq!(pump.tick(false), None);
        pump.on_tts(TtsEvent::Started { id: 2 });
        pump.on_tts(TtsEvent::Failed {
            id: 2,
            error: "no voice".into(),
        });
        assert_eq!(pump.tick(false), Some(SessionEvent::Speaking(false)));
    }

    #[test]
    fn stop_drops_the_queue_and_speaking_ends_when_flushed() {
        let mut pump = Pump::new();
        pump.on_action(&Action::Speak("One.".into()));
        pump.on_action(&Action::Speak("Two.".into()));
        pump.on_tts(TtsEvent::Started { id: 1 });
        assert_eq!(pump.on_action(&Action::Stop), None);
        // Until the worker says `Stopped`, it may still be pushing.
        assert_eq!(pump.tick(false), None);
        assert_eq!(pump.on_tts(TtsEvent::Stopped { id: 1 }), None);
        assert_eq!(pump.tick(false), Some(SessionEvent::Speaking(false)));
    }

    #[test]
    fn a_stop_while_idle_ends_speaking_once_the_ring_is_flushed() {
        let mut pump = Pump::new();
        pump.on_action(&Action::Speak("Long reply.".into()));
        pump.on_tts(TtsEvent::Started { id: 1 });
        pump.on_tts(TtsEvent::Finished {
            id: 1,
            audio_ms: 5000,
            synth_ms: 400,
        });
        pump.on_action(&Action::Stop);
        assert_eq!(pump.tick(true), None);
        assert_eq!(pump.tick(false), Some(SessionEvent::Speaking(false)));
    }

    #[test]
    fn loading_events_are_reported() {
        let mut pump = Pump::new();
        assert_eq!(
            pump.on_stt(SttEvent::Loaded { load_ms: 900 }),
            vec![SessionEvent::SttLoaded { load_ms: 900 }]
        );
        assert_eq!(
            pump.on_tts(TtsEvent::Loaded {
                voices: vec!["a".into(), "b".into()],
                load_ms: 300
            }),
            Some(SessionEvent::TtsLoaded {
                load_ms: 300,
                voices: 2,
                names: vec!["a".into(), "b".into()],
            })
        );
        assert_eq!(
            pump.on_stt(SttEvent::Speed {
                step_ms: 30.5,
                dropped: 0
            }),
            vec![SessionEvent::Speed {
                step_ms: 30.5,
                dropped: 0
            }]
        );
    }

    #[test]
    fn vocab_text_is_one_clean_phrase_per_line() {
        let words: Vec<String> = [
            "claude",
            " codex ",
            "",
            "claude",
            "# not a comment",
            "two\nlines",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (text, n) = vocab_text(&words);
        let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(lines, vec!["claude", "codex", "two lines"]);
        assert_eq!(n, 3);
    }

    #[test]
    fn write_vocab_creates_the_folder_and_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app").join("vocab.txt");
        assert_eq!(
            write_vocab(&path, &["claude".into(), "hr".into()]).unwrap(),
            2
        );
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.ends_with("claude\nhr\n"), "{first}");
        assert_eq!(write_vocab(&path, &["codex".into()]).unwrap(), 1);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .ends_with("\ncodex\n")
        );
        assert!(!path.with_extension("txt.tmp").exists());
    }
}
