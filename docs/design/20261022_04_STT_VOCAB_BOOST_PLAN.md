---
title: Boosting a vocabulary list in Kyutai STT
date: 2026-10-22
status: Draft
covers:
  - A cargo feature `stt-moshi-xn-boost`, on top of `stt-moshi-xn`, that
    biases the STT model's text tokens toward the phrases of a list
  - SentencePiece encoding in `src/spm.rs` (unigram Viterbi over the
    model's own piece scores), to turn the phrases into token ids
  - A `TokenBooster` that follows the phrases' token sequences as the model
    writes them
  - The first list: the `lib*` folders in `~/src`
  - An A/B test on the desktop: Phonon says sentences with those names,
    and STT transcribes them with and without the boost
sources:
  - Owner's request (2026-10-22): "try giving it the list of lib* in ~/src
    with stt-*-boost feature".
  - xn-moshi 0.2.6 `asr.rs`: `trait TokenBooster { fn on_text_token(&mut
    self, u32); fn text_biases(&self) -> Vec<(u32, f32)>; }`, attached with
    `AsrState::reset_batch_idx(batch, temp, cond, Some(booster), delay)`.
    `on_text_token` is called with every text token after the ASR delay.
    The biases are added to the text logits before each step, and only
    after the delay. Tokens 0 (end of paragraph), 2 (end of stream),
    3 (pad) and 4 (silence pad) end a word.
  - `ls -d ~/src/lib*` on 2026-10-22: libbarstool, libcontrols, libdevice,
    libgenai, libmobile, libmorph, libmsdf, libshader, libtypedmd.
  - The SentencePiece `ModelProto`: field 1 repeats `SentencePiece {
    piece = 1, score = 2 (float), type = 3 }`. Unigram encoding is the
    segmentation with the largest total score, with `▁` for spaces and a
    leading `▁`.
related:
  - docs/design/20261022_02_STT_MOSHI_XN_PLAN.md
---

# Boosting a vocabulary list in Kyutai STT

## 1. Goal

STT writes "libmorph" and the other library names as written, not as
sound-alikes ("lib morph", "Live Morph"). The list is a text file, one
phrase per line. The boost is a feature, and without it nothing changes.

## 2. Decisions

1. **`stt-moshi-xn-boost = ["stt-moshi-xn"]`.** It adds no dependency.
2. **The list file is `vocab.txt`.** On the desktop it is read from the
   working directory (or `--vocab <file>`), and on Android from
   `<data dir>/vocab.txt`. `scripts/vocab-lib.sh` writes the repository's
   `vocab.txt` from `ls -d ~/src/lib*`, and `scripts/push-vocab.sh` copies
   it to the phone. Each phrase is boosted both as written and with a
   capital first letter, since the model capitalizes sentence starts.
3. **Encoding:** `spm::Model` keeps each piece's score. `encode(text)` is
   the unigram Viterbi segmentation of `▁` + text, with spaces as `▁`.
   Pieces are matched up to 16 characters long, and a character no piece
   covers falls back to its `<0xNN>` byte pieces.
4. **The booster** is a token trie of every phrase variant:
   - At a word start (the last token ended a word, or nothing has been
     written yet), the first token of every phrase gets `start` (default
     1.5).
   - Inside a phrase that has matched so far, its next token gets `cont`
     (default 4.0).
   - A token that does not continue any match drops those matches. A word
     end clears them.
   - `VOICEXN_BOOST=start,cont` overrides both values, for tuning.
5. **Attached once** after `Stt::load`, through a new
   `AsrStep::set_booster`, which calls `reset_batch_idx(0, None, None,
   Some(booster), None)`.

## 3. Steps

1. **SentencePiece encoding.** Scores in the parsed pieces, `encode`, and
   tests: a piece table built in the test, where the best segmentation
   wins over a greedy one; byte fallback; and, with the real tokenizer,
   `decode(encode(s)) == s` for the library names and an English
   sentence.
   **Done when:** the tests pass.
2. **The booster and the list.** The trie booster with unit tests (start
   bias at word starts only, continuation along a match, reset on a
   mismatch and at a word end), `vocab.txt` from `scripts/vocab-lib.sh`,
   and loading in `Stt::load`.
   **Done when:** the tests pass, and in the desktop A/B (Phonon WAVs of
   three sentences that use the nine names, through `--transcribe` built
   with and without the feature) the boosted build writes more of the
   names correctly. Both transcripts are logged. Words outside the list
   must come out the same.
3. **On the phone.** `push-vocab.sh`, and a spoken check with the owner.
   **Done when:** the S23 writes "libmorph" for "libmorph" with the VAD
   build plus the boost.

## 4. Open questions

1. Are 1.5 and 4.0 right? Step 2's A/B gives the first measure.
