---
title: Mimi's encoder from the STT GGUF, without mimi.safetensors
date: 2026-10-22
status: Implemented
covers:
  - Loading Kyutai STT's Mimi encoder from `model.q4k.gguf`, which already
    holds it, so `mimi.safetensors` (385 MB) is no longer read, pushed or
    kept in memory
  - An encoder-only load in xn-moshi (`Mimi::load_encoder`), on a fork
    pinned by git until upstream releases it
  - libvoicexn's `Stt::load`, `scripts/push-stt.sh` and the README
  - libvoicexn 0.2.0
  - Not covered: re-quantizing the model, Mimi's decoder for any other
    use, the q4k speed tuning (its own plan)
sources:
  - Owner (2026-10-22): "we should fix the decoder issue if it requires such
    as large file to be pushed to mobile as well, but that can be a new plan
    after this"; "write the encoder plan, see if there's a crate we can use
    instead of writing it".
  - xn-moshi 0.2.6 (crates.io, the latest) and gradium-ai/xn-moshi `main`
    at `b5366b7` (the revision `scripts/fetch-stt.sh` pins, no newer
    commits): `src/mimi.rs`, `src/asr.rs`, `examples/quantize.rs`.
  - `model.q4k.gguf` read with a GGUF header reader: 344 tensors, of which
    209 are `mimi.*` in f32 (`mimi.downsample` 1, `mimi.encoder` 28,
    `mimi.encoder_transformer` 80, `mimi.quantizer` 100).
  - `mimi.safetensors`: `decoder` 28 tensors, 58.9 MB; `decoder_transformer`
    80, 100.8 MB; `upsample` 1, under 0.1 MB; and the encoder side,
    `encoder` 50.5 MB, `encoder_transformer` 100.8 MB, `downsample` 4.2 MB,
    `quantizer` 69.5 MB.
  - crates.io search for `mimi`, `moshi` and `kyutai` (2026-10-22).
related:
  - docs/design/20261022_02_STT_MOSHI_XN_PLAN.md
  - docs/design/20261022_05_LIBVOICEXN_SPLIT_PLAN.md
---

# Mimi's encoder from the STT GGUF, without mimi.safetensors

## 1. Goal

STT loads Mimi, Kyutai's audio codec, from `mimi.safetensors` (385 MB,
f32), because xn-moshi's `Mimi::load` builds the decoder as well as the
encoder. Speech-to-text only encodes: `Asr` calls `init_encode_state` and
`encode_step`, never the decoder. The quantized GGUF already carries every
encoder-side tensor in f32. After this plan, `Stt::load` reads Mimi from the
GGUF, the phone gets one 1.02 GB model file instead of 1.41 GB, and about
160 MB of decoder weights are no longer held in memory.

## 2. Findings

1. **No crate does this on xn.** The Mimi implementations on crates.io are
   Kyutai's `moshi` 0.6.4 (candle), `rlx-mimi` 0.2.11 (RLX) and
   `eredu-codec` 0.5.0 (its own backends). Each would put a second tensor
   framework beside xn, and none fits xn-moshi's `Asr::new(delay, temp,
   mimi: Mimi<f32, B>, lm)`, which takes xn-moshi's own `Mimi`. xn-moshi
   0.2.6 is the newest release, and its `main` (`b5366b7`) has no
   encoder-only load.
2. **The GGUF has the encoder.** `examples/quantize.rs` skips names that
   start with `mimi.decoder` (which also matches `mimi.decoder_transformer`)
   or `mimi.upsample.`, and quantizes only `lm.transformer.layers.*`
   linears. The Mimi tensors it keeps are written as they are, f32. Every
   GGUF made by `scripts/fetch-stt.sh` (q4k, q4_0, q8) has them.
3. **Nothing on our side can stand in for the missing tensors.** `xn`'s
   `VB` loads one GGUF or a set of safetensors, not both, so the decoder
   cannot be filled from a stub file beside the GGUF. Rewriting `Asr`
   (615 lines) around our own encoder would copy most of xn-moshi.
4. **The change in xn-moshi is small.** `Mimi`'s fields are private, and
   only `init_decode_state`, `decode` and `MimiDecodeState::decode_step`
   use the decoder side.

## 3. Decisions

1. **Write it in xn-moshi, not here.** Added to `src/mimi.rs`, and nothing
   else changed:

   ```rust
   struct MimiDecoder<T: WithDTypeF, B: Backend> {
       decoder: seanet::SeaNetDecoder<T, B>,
       decoder_transformer: bt::BatchedProjectedTransformer<xn::Unquantized<T, B>>,
       upsample: conv::ConvTrUpsample1d<T, B>,
   }

   pub struct Mimi<T: WithDTypeF, B: Backend> {
       encoder, encoder_transformer, downsample, quantizer, config,  // as now
       decoder: Option<MimiDecoder<T, B>>,                            // replaces the three fields
   }

   impl<T: WithDTypeF, B: Backend> Mimi<T, B> {
       pub fn load(vb: &Path<B>, cfg: Config) -> Result<Self>;          // unchanged: decoder Some
       /// Only what encoding needs: `encoder`, `encoder_transformer`,
       /// `downsample` and `quantizer` under `vb`.
       pub fn load_encoder(vb: &Path<B>, cfg: Config) -> Result<Self>;  // decoder None
       pub fn has_decoder(&self) -> bool;
   }
   ```

   `init_decode_state` and `decode` return
   `Err(xn::Error::Msg("mimi: loaded without its decoder".into()))` when it
   is `None`; `MimiDecodeState` can then not exist without a decoder, so
   `decode_step` needs no check. The change is additive for xn-moshi's
   users.

2. **Hosting.** A fork, `github.com/tmzt/xn-moshi`, from `b5366b7`, branch
   `mimi-encoder-only`, with the change as one commit and offered upstream
   as a pull request to `gradium-ai/xn-moshi`. libvoicexn depends on it by
   git and revision, as it does on `ptts`:

   ```toml
   xn-moshi = { git = "https://github.com/tmzt/xn-moshi", rev = "<commit>", optional = true }
   ```

   Not `[patch.crates-io]`: a patch applies only to this workspace, not to
   apps that depend on libvoicexn. When upstream releases the change, a
   later version returns to crates.io.

3. **libvoicexn loads Mimi from the GGUF.** In `Stt::load`, the GGUF's `VB`
   gives two paths, `root.pp("lm")` for the LM as now and `root.pp("mimi")`
   for `Mimi::load_encoder`. `mimi.safetensors` is not opened, and its
   absence is not an error. Nothing else in `stt.rs` changes: `Asr::new`
   gets the same `Mimi` type.

4. **Files.** `scripts/push-stt.sh` pushes `model.q4k.gguf`, `config.json`
   and `tokenizer.model` (1.02 GB). `scripts/fetch-stt.sh` still downloads
   `mimi.safetensors`, which `quantize` reads, and keeps it in the model
   store. The README's model section and push size follow.

5. **Version 0.2.0.** libvoicexn's public API exposes xn-moshi:
   `VocabBooster` implements `xn_moshi::asr::TokenBooster`. Moving
   xn-moshi from crates.io to a git source makes that trait a different
   type for an app that also depends on crates.io's xn-moshi, so the
   release is not strictly additive (AGENTS.md Rule 5.4 and 5.5), and
   `VERSIONS.md` marks it **Breaking**. The return to crates.io later is
   another such release.

## 4. Steps

0. **The fork.** The owner creates `tmzt/xn-moshi` from
   `gradium-ai/xn-moshi` (or agrees that it is created with `gh repo fork`),
   and it is cloned to `~/src/xn-moshi`.
   **Done when:** `git -C ~/src/xn-moshi log -1` shows `b5366b7` on `main`.

1. **`Mimi::load_encoder` in xn-moshi.** Decision 1, on branch
   `mimi-encoder-only`, with a test that builds the encoder side from a
   tiny `Config` (one SEANet ratio, one transformer layer, two codebooks)
   whose weights are written to an in-memory safetensors buffer without any
   `decoder`, `decoder_transformer` or `upsample` names, and checks that
   `Mimi::load` fails on it, `load_encoder` succeeds, `encode` returns codes
   of the right shape, and `decode` returns the error.
   **Done when:** `cargo fmt --check`, `cargo clippy --all-targets` (no new
   warnings) and `cargo test` pass in `~/src/xn-moshi`; the branch is
   pushed; and the pull request to `gradium-ai/xn-moshi` is opened if the
   owner wants it.

2. **libvoicexn on the fork.** Decisions 2 and 3.
   **Done when:** libvoicexn passes fmt, clippy and tests; `cargo tree -p
   libvoicexn --features stt-moshi-xn -i xn-moshi` shows the git source; and
   on the Mac, with `mimi.safetensors` moved out of `models/stt-1b-en_fr`,
   `voicexn --transcribe` of the three plan 04 sentences gives the same text
   as before the change, with load time and mean step logged to
   `out/stt/`.

3. **Files and the phone.** Decision 4 in `scripts/push-stt.sh` and the
   README, each in its own commit.
   **Done when:** on the S23, after `adb shell run-as dev.tmzt.voicexn rm
   files/stt-1b-en_fr/mimi.safetensors`, the APK built at step 2 loads STT
   and a tap-to-talk turn is heard and repeated; and the log has the load
   time and `dumpsys meminfo dev.tmzt.voicexn` total PSS after load, for
   this build and the one before it.

4. **0.2.0.** Decision 5: `version = "0.2.0"`, the `VERSIONS.md` entry
   (xn-moshi 0.2.6 from crates.io to the fork's revision, **Breaking**;
   `Stt` no longer needs `mimi.safetensors`), and the tag `v0.2.0` on that
   commit.
   **Done when:** `git tag` shows `v0.2.0` on the version commit.

## 5. Open questions

1. Is the git-source move breaking (decision 5)? This plan says yes,
   because a public trait comes from xn-moshi. If the owner judges that no
   app will mix the two sources, it could be 0.1.1.
2. Should the fork be offered upstream before libvoicexn uses it, and
   should libvoicexn wait for a release instead of using the fork?

## Amendment — 2026-10-22 14:32 EDT: approved; implementation started

The owner (2026-10-22): "create the fork, no pull request (I will make that
decision and write it up myself). approve and implement." This replaces
the open questions of section 5 and the pull request in step 1:

- The fork `github.com/tmzt/xn-moshi` was created with `gh repo fork`
  and cloned to `~/src/xn-moshi` at `b5366b7` (step 0 holds).
- No pull request is opened to `gradium-ai/xn-moshi`; that is the owner's
  decision. libvoicexn uses the fork now (open question 2).
- The version is 0.2.0, marked **Breaking**, as decision 5 says (open
  question 1).

`status` is now `In implementation`, and the document is immutable from
here on (AGENTS.md Rule 1.5). Step 1 is in the fork, not in this
repository, so the status change is its own commit here.

## Amendment — 2026-10-22 14:53 EDT: implemented

Every step is done and its **Done when:** holds, with the departures
recorded in `CHANGES.md` ("Mimi encoder-only, steps 1 to 4"). `status` is
now `Implemented`.

- Step 0: the fork `tmzt/xn-moshi`, cloned at `b5366b7`.
- Step 1: `5d994ac` in the fork, branch `mimi-encoder-only`, pushed; no
  pull request, by the owner's decision.
- Step 2: `b6480aa`.
- Step 3: `6390507`, `f2a3023`.
- Step 4: `32b8032` (tag `v0.2.0`).
- Record: `d4f66ae`.

Measured: the same transcripts on the desktop, 385 MB less to push
(1.02 GB), and 152 MB less memory on the S23 after load.
