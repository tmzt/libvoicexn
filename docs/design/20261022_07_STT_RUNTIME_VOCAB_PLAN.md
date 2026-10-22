---
title: Changing the STT vocabulary boost while the model runs
date: 2026-10-22
status: Draft
covers:
  - Replacing the boosted phrase list of a running `SttWorker`, under
    `stt-moshi-xn-boost`: a new `SttRequest::Vocabulary(Vec<String>)` and a
    new `SttEvent::Vocabulary { phrases, sequences }`
  - Encoding the phrases off the stt thread, and applying them only between
    utterances, in both the tap-to-talk and the always-listening loop
  - `voicexn-tty` input from a WAV file and `vocab:` lines on stdin, to show
    and measure it
  - libvoicexn 0.3.0
  - Not covered: tuning the boost, the vocabulary's source (handlr's
    companion protocol), a UI for it in `examples/voicexn`
sources:
  - handlr's session (2026-10-22, relayed with the owner's approval):
    handlr's companion protocol sends a vocabulary list (agent, project and
    workspace names from the herdr snapshot) in `hello`'s result and again in
    `vocabulary.changed`; `apps/companion` (from examples/voicexn) and
    `apps/companion-tty` (from examples/voicexn-tty) feed it to libvoicexn's
    STT, pinned to a tag. It asks for a runtime replacement under
    `stt-moshi-xn-boost` that does not stall the 80 ms step, takes effect from
    the next utterance, confirms with an event and a phrase count, and clears
    the boost on an empty list. Lists are tens to a few hundred short
    phrases. A new variant on an exhaustive public enum is a minor bump
    (Rule 5.4), which handlr expects.
  - handlr's `docs/design/20261022_04_COMPANION_APP_PLAN.md`, amendment of
    2026-10-22 16:23 EDT: until this lands, handlr rewrites `vocab.txt` and it
    is read at the next start.
  - libvoicexn at `595197e` (v0.2.0 is `32b8032`): `src/boost.rs`
    (`VocabBooster`), `src/stt.rs` (`Stt::load`, `Listener::feed`, `run`,
    `run_gated`, `SttWorker`, `SttRequest`, `SttEvent`).
  - xn-moshi at `5d994ac` (`src/asr.rs`): `TokenBooster { fn
    on_text_token(&mut self, u32); fn text_biases(&self) -> Vec<(u32, f32)> }`,
    held per batch item as `Option<Box<dyn TokenBooster>>`;
    `AsrState::reset_batch_idx` is the only way to set it, and it also
    resets the LM's and Mimi's streaming state; `text_bias` builds a
    `(batch, 8000)` bias tensor on every step while a booster is attached.
related:
  - docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md
  - docs/design/20261022_03_VAD_TOUCH_GATE_PLAN.md
  - docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md
  - docs/design/20261022_06_MIMI_ENCODER_ONLY_PLAN.md
---

# Changing the STT vocabulary boost while the model runs

## 1. Goal

An app can hand a running `SttWorker` a new phrase list and have the boost
use it from the next utterance, without restarting the model, without a
longer step, and with an event that says when it took effect and how many
phrases it holds. An empty list turns the boost off. The phrase list in
`SttConfig::vocab` stays as the list at load.

## 2. Decisions

1. **The interface is a request and an event**, beside `Arm` and `Close`:

   ```rust
   #[derive(Clone, Debug)]            // no longer Copy: it carries a Vec
   pub enum SttRequest {
       Arm,                           // as now, in every STT build
       Close,
       /// Replaces the boosted phrases from the next utterance on; an empty
       /// list turns the boost off.
       #[cfg(feature = "stt-moshi-xn-boost")]
       Vocabulary(Vec<String>),
   }

   pub enum SttEvent {
       // ... as now
       /// A phrase list took effect: at load from `SttConfig::vocab`, and
       /// after each `SttRequest::Vocabulary`. `sequences` counts the token
       /// sequences boosted (each phrase as written and capitalized).
       #[cfg(feature = "stt-moshi-xn-boost")]
       Vocabulary { phrases: usize, sequences: usize },
   }
   ```

   `SttWorker::send` exists with `vad-earshot` or `stt-moshi-xn-boost`. The
   phrases are trimmed and empty ones dropped; there are no `#` comments,
   which only `vocab.txt` has. `Arm` and `Close` are not changed.

2. **The booster is swapped in place, never through `reset_batch_idx`.**
   That call also clears the LM's KV cache and Mimi's encoder state, which
   mid-stream would lose up to 60 s of context. Instead `VocabBooster`
   keeps its phrase table and a shared slot,
   `Arc<Mutex<Option<BoostTable>>>`, for the next one. In `on_text_token` it
   takes a table waiting in the slot (`try_lock`, then `take`), replaces its
   own and clears the matches in progress; the stt thread holds the other
   end. The lock is taken by the stt thread only, once per token, so it is
   never contended. xn-moshi is not changed, and `VocabBooster`'s public
   functions keep their signatures.

3. **A booster is always attached under `stt-moshi-xn-boost`**, with an
   empty table when there is no list at load, so a list can arrive later.
   While one is attached, xn-moshi adds an 8000-wide bias tensor on every
   step. Step 4 measures what that costs; if the mean step grows by more
   than 0.2 ms on the S23, an empty table is detached at the next quiet
   point through `reset_batch_idx` instead, when the gate is idle or after
   an utterance has ended.

4. **Encoding is off the stt thread.** The `spm::Decoder` the model loads
   is kept in an `Arc`. A `Vocabulary` request spawns a short thread, named
   `stt-vocab`, which runs `VocabBooster::from_phrases`'s encoding and sends
   the table back over a channel with the request's sequence number. The
   stt thread only `try_recv`s it. If several lists are in flight, the one
   requested last wins, and older ones arriving after it are dropped.

5. **It takes effect only between utterances.** A table that has arrived
   waits in `Listener` as `next_vocab` until the next boundary:
   - with `vad-earshot`: at once if the gate is `Idle`, otherwise when the
     gate next becomes `Idle`;
   - always listening: at once if the segmenter holds no words, otherwise
     right after it ends the current utterance (`Segmenter::push` returning
     `Some`);
   - before the model has loaded: right after the load, in place of the
     list from `SttConfig::vocab`.
   Applying it puts the table in the slot (decision 2) and sends
   `SttEvent::Vocabulary`. The booster picks it up on the next token, which
   comes after the boundary.

6. **The example shows it.** `voicexn-tty` gets:
   - `--input <wav>`: reads the file (mono or stereo, any rate) and pushes
     it through `Mic::push` in 20 ms blocks at real time, instead of the
     default input device; it quits 2 s after the file ends;
   - a stdin line starting `vocab:` sends `SttRequest::Vocabulary` with the
     comma-separated names after it (`vocab:` alone clears it); any other
     line does what Enter does now;
   - it prints `stt: vocabulary <phrases> phrases (<sequences> sequences)`
     on `SttEvent::Vocabulary`.

   `examples/voicexn` only handles the new event in its log.

7. **Version 0.3.0.** `SttRequest` gains a variant and loses `Copy`, and
   `SttEvent` gains a variant: both are exhaustive public enums, so the
   release is not strictly additive (AGENTS.md Rule 5.4). `VERSIONS.md`
   marks the `Copy` removal and the new variants **Breaking**.

## 3. Steps

1. **The swappable booster.** Decision 2 in `src/boost.rs`: `BoostTable`
   (the phrase sequences), the slot, and `VocabBooster::with_slot` for the
   stt thread's end. Tests: a table in the slot replaces the old one on the
   next token and clears a match in progress; an empty table gives no
   biases; the old list still boosts until the new one is taken.
   **Done when:** libvoicexn passes fmt, clippy and tests with and without
   `stt-moshi-xn-boost`.

2. **The request, the event and the boundaries.** Decisions 1, 3, 4 and 5
   in `src/stt.rs`. The boundary logic is a small type of its own,
   `VocabSwap { next: Option<(u64, BoostTable)>, latest: u64 }` with
   `offer(seq, table)` and `take()`, tested apart from the model: the
   latest request wins, an older one arriving late is dropped, and nothing
   is taken mid-utterance.
   **Done when:** fmt, clippy and tests pass for every feature set, and
   `examples/voicexn` and `voicexn-tty` build with the new event.

3. **`voicexn-tty` on a file.** Decision 6.
   **Done when:** it builds with and without `vad-earshot`, and
   `voicexn-tty --input <wav>` prints the same utterances for the three
   plan 04 sentences as `voicexn --transcribe` does for each file.

4. **Measured on the desktop and the phone.**
   - M4: one WAV of the three plan 04 sentences with 2 s of silence between
     them, through `voicexn-tty --input`, built without `vad-earshot`, from
     an empty list, with `vocab: <the nine lib* names>` sent on stdin while
     the first sentence is still being heard. The first sentence must come
     out as it does with no list; the event must arrive after it, with 9
     phrases and 18 sequences; the second and third must come out as they
     do with the list loaded at start. Then `vocab:` alone, and a fourth
     pass of the first sentence comes out plain. The mean step with an
     empty booster attached is logged against a build without the feature.
   - S23: the APK with `vad-earshot stt-moshi-xn-boost` loads `vocab.txt`,
     sends `SttEvent::Vocabulary` (9, 18), and a tap-to-talk turn is heard
     and repeated; the mean step is logged against v0.2.0's.
   **Done when:** all of that holds, with the logs in `out/stt/` and an
   entry in `out/LOG.md`.

5. **0.3.0.** Decision 7: the version, the `VERSIONS.md` entry and the tag
   `v0.3.0`, pushed with the branch.
   **Done when:** `git ls-remote origin` shows `v0.3.0` on the version
   commit.

## 4. Open questions

1. Should `SttRequest::Vocabulary` also take the biases, so an app can
   change `start` and `cont` at runtime? This plan keeps them from
   `SttConfig::boost`.
2. Is "from the next utterance" right in always-listening mode, where an
   utterance can run long? A list that arrives mid-sentence waits until
   about 1.2 s of no new words.
3. Should the event also report the list as encoded, for example the
   phrases that encoded to nothing? Today such a phrase is only dropped.
