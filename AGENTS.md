# AGENTS.md — voicexn: a minimal realtime STT/TTS example on Android

voicexn is an application, not a library: a `libmorph` app (with `libmsdf`,
and `libcontrols` where it earns its place) that listens through the
microphone, transcribes speech and speaks text back, with the models run
on-device through xn rather than ONNX Runtime. The rules below are carried
over from libcontrols' `AGENTS.md`.

## CRITICAL RULE 0: THE CHANGES.MD PROTOCOL

Before writing, modifying or deleting code for a new planned work item:

1. Only planned new work, feature plans and architectural changes are
   appended to the bottom of `CHANGES.md`. Routine implementation work that
   follows an existing plan does not need its own addendum.
2. Each addendum is timestamped (`## YYYY-MM-DD HH:MM TZ — <subject>`, from the
   system clock) and names the plan it belongs to. Entries are appended, never
   edited.
3. Departures from a frozen plan (Rule 1.7) are recorded as addenda here.
4. Updates are small, incremental steps. No large, unannounced redesigns of
   the codebase.

## CRITICAL RULE 1: SPECS AND PLANS

1. Specs and plans live in `docs/design/`, named `YYYYMMDD_NN_NAME.md`. `NN` is
   the sequence within the day, and plans end in `_PLAN`. Each starts with YAML
   front matter: `title`, `date`, `status`, what it covers, its sources, and
   related or dependent documents.
2. Plans are written in plain engineering language that a person could build
   from alone: concrete interfaces, data layouts, numbers and paths, and
   numbered steps, each ending with **Done when:** stating what works at that
   point.
3. `status` is one of:
   - `Draft`: being worked out, changed only by amendments;
   - `Hold`: no work may start or continue until an amendment returns it to
     `Draft`;
   - `In implementation`: frozen.
   - `Implemented`: every step is done and its **Done when:** holds. It stays
     frozen, and living sections stay open. It is set by an amendment that lists
     the commits implementing each step, in the commit that completes the last
     step or in a later one.
   - `Superseded (by <plan>)`: replaced before implementation by the named
     plan, which carries its content forward. Nothing in it is implemented.
     It is set with an amendment naming the replacement and what moved where.

   The front-matter `status` is the only part of a document edited in place,
   and every status change gets its own amendment.
4. An amendment is appended to the end of the document as
   `## Amendment — YYYY-MM-DD HH:MM TZ: <subject>`. The body is never rewritten.
   An amendment states which earlier text it replaces; where they disagree, the
   later amendment wins. Timestamps come from the system clock, never typed by
   hand, and a wrong one is corrected by a later note, not by editing.
5. The first commit that implements a plan sets `status: In implementation`
   and appends an amendment saying implementation has started and the document
   is now immutable. After that it changes only on the owner's explicit
   instruction.
6. A plan may name **living sections** in its front matter. These stay open
   after implementation starts: new entries are appended as amendments, and an
   entry is fixed once its item is implemented. Implementation records, such as
   which commits implemented an entry, are appended as amendments, not filled in
   place.
7. When implementation departs from a frozen plan:
   - small departures needed to make the plan work are recorded in an addendum
     to this file, naming the plan, the step and the reason;
   - a departure that changes behaviour or an interface others depend on is
     discussed with the owner first (Rule 2), then recorded;
   - a frozen plan is never edited to match the code. A larger change of
     direction becomes a new dated plan that names the one it replaces.

## CRITICAL RULE 2: DISCUSS BEFORE CHANGING A PLAN

1. A change to a spec or plan is proposed to the owner first: what changes,
   where and why. Nothing is written to the document until the owner agrees.
2. Refinements made during the discussion go into the proposal, not the file.
   Once agreed, the whole change is written as **one** amendment, not a series
   of amendments that replace one another.

## CRITICAL RULE 3: COMMITS AND EVIDENCE

1. At most one crate or subproject per commit. Repository-level files go in
   their own commits: this file, the workspace manifest, evidence logs, tooling
   config and scripts. The one exception is adding a crate to the workspace in
   that crate's first commit.
2. A plan's status change and its immutable marker go in the commit that starts
   implementing it.
3. Commits name their paths explicitly (`git add <paths>`,
   `git commit -- <paths>`), so unrelated work in progress, including another
   agent's, is never swept in. Commits already on the shared branch are never
   amended or rewritten.
4. Before each code commit, the crate passes `cargo fmt -p <crate> -- --check`,
   `cargo clippy -p <crate> --all-targets` with no new warnings, and
   `cargo test -p <crate>`.
5. Renders and other generated evidence go under the gitignored `out/`. Each
   area keeps its own log file. Entries are appended under dated headings, with
   `./`-relative links and descriptions that stand on their own without the
   images. Earlier entries are never edited.

## CRITICAL RULE 4: DECISION STATUS IN THE README

1. `README.md` ends with an `## Architectural decisions` section: one GitHub
   Markdown table listing every spec and plan in `docs/design/`, oldest first.
   Nothing follows the table.
2. Columns: **Document** (the title, as a relative link such as
   `[Camera view and capture](docs/design/20261014_01_CAMERA_VIEW_AND_CAPTURE_PLAN.md)`;
   no absolute or GitHub URLs), **Date** (from its front matter), **Status**
   (exactly its front-matter `status`, including `Superseded (by <link>)`),
   and **Decision** (one line: what it decides, and why it is at that status).
3. A row is added in the commit that adds the document, and updated in the
   same commit as every status-change amendment. This is an exception to
   Rule 3.1. The table is a summary: the documents and their amendments stay
   the record.

## CRITICAL RULE 5: VERSIONS AND TAGS

1. The commit that changes the crate's `version` in `Cargo.toml` is tagged
   `v<version>` (for example `git tag v0.3.0 <commit>`) as soon as it is
   committed. A tag that has been pushed is never moved or deleted.
2. `VERSIONS.md` records each version change, and only version changes:
   one entry per version, appended oldest first, headed
   `## <version> — YYYY-MM-DD (<short commit>, tag v<version>)`. The entry
   lists the public API changes since the previous version: what was added,
   changed or removed, with breaking changes marked **Breaking**. Internal
   changes are not listed, and commits that do not change the version get no
   entry. Earlier entries are never edited.
3. The `VERSIONS.md` entry is in the same commit as the version change. This
   is an exception to Rule 3.1.
4. While the version is `0.x`, the bump follows the public API change. A
   release whose public API changes are not strictly additive bumps the minor
   version (`0.x.y` to `0.(x+1).0`): anything removed, renamed or changed in
   signature or type, behaviour a caller can observe changing, or a variant or
   field added to an exhaustive public enum or struct. A release whose changes
   are strictly additive, or change no public API, bumps the patch version
   (`0.x.y` to `0.x.(y+1)`). This applies from 0.3.0 on; earlier versions are
   not re-labelled.
5. The public API rule is recursive through dependencies. When a crate with
   a public API moves a dependency to a version that is not semver-compatible
   with the one it used (for a `0.x` dependency, a new minor version; from
   1.0, a new major version), its own release is not strictly additive and
   bumps its minor version under Rule 5.4 (its major version from 1.0), even
   when its own code does not change. Every crate with a public API that
   depends on it does the same when it takes that release, and so on up the
   chain. The `VERSIONS.md` entry names the dependency with its old and new
   versions and marks the change **Breaking**. A repository with no public
   API (an application) only updates its manifest and lock file.
