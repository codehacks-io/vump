# Working in this repository

vump keeps semver version numbers in sync across a repository's files, and
verifies in CI that a released tag matches what is recorded in source. One
crate, ports and adapters.

## Where things are written down

| Document | Holds |
| --- | --- |
| [`DESIGN.md`](DESIGN.md) | How vump behaves and why. Authoritative: code that disagrees with it means one of the two is defective. |
| [`ENGINEERING.md`](ENGINEERING.md) | How code is written and judged here. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | How work flows: branches, commits, pull requests, releases. |
| [`BACKLOG.md`](BACKLOG.md) | What is not built yet, and what was decided against. |
| [`README.md`](README.md) | The user-facing manual. |
| [`sandbox/`](sandbox) | Working projects per ecosystem, to try by hand and to read as examples. |
| [`website/`](website) | The landing page, and what its deployment still needs. Not part of the crate, and not versioned with it. |

Before changing behavior, read the relevant `DESIGN.md` section. Before
building something new, check `BACKLOG.md` — it may already be decided against,
and the entry says why. Proposing one of those again is fine, but say what
changed.

## The rules that are easiest to get wrong

Each is stated in full, with its reasoning and a worked example, where the
table above says it lives — the code standards in [`ENGINEERING.md`](ENGINEERING.md),
the design rules in [`DESIGN.md`](DESIGN.md).

- **A type says what a value means.** If a field's meaning depends on which
  caller produced it, the type is wrong. An `Option` is genuine absence, never
  a variant in disguise.
- **Derive what can be derived.** Two stored fields that must agree will
  eventually disagree.
- **Refuse rather than guess.** When vump cannot know, it fails with an
  actionable message. It never picks the likely option.
- **One `vump.toml` per repository.** Several things versioning separately is
  what `[[project]]` is for. A second configuration deeper in the same
  repository is the mistake that feature exists to prevent, not a layout to
  accommodate — do not weigh the convenience of anyone using one. The sandbox
  is the sole deliberate exception, and it pays for it.
- **Comments explain why, not what** — and never how the code came to be. No
  development history, no "previously", no narration of a discussion.
- **Tracked files describe the tool, nothing else.** This repository is
  public. Nothing about who works on it, how the work is divided, what tooling
  or hardware sits on a contributor's machine, or how anyone's credentials are
  held belongs in a file that ships with it.
- **A fixed bug arrives with the test that would have caught it.**
- **A document earns its length.** Record the reasoning — the constraint, the
  measurement, the option rejected — never the investigation that reached it.

## Before pushing

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs exactly this on Linux, macOS and Windows. Running it locally first is
much faster than waiting for a runner to report the same thing.

`./scripts/sandbox-smoke.sh` is the separate check: it bumps every sandbox and
asks each ecosystem's own toolchain what version it builds, which is the one
thing the suite cannot answer. It needs node, dotnet, uv and cargo, skips what
is missing, and is not part of CI for that reason — run it when a format's
reading or writing changed.

## Working agreement

- **Open a pull request when a coherent set of work is done, then pause for
  review.** A pull request is a checkpoint, not a formality.
- **Propose the next version when a pull request is opened, in the reply and
  never in the pull request itself.** Opening one is where a change's maturity
  becomes legible, so name the version it should be tagged as, justify it in a
  line or two against the rule in [`CONTRIBUTING.md`](CONTRIBUTING.md), and give
  the exact command in its own fenced `bash` block so it can be run without
  being retyped. Deciding this is not work to hand back.

  It stays out of the description because a description outlives the decision:
  it says what the change is, whereas a suggested version is advice about what
  to do next, and once a tag exists — or a different one is chosen — the
  suggestion is at best noise and at worst wrong.
- **Commit often.** Every commit compiles and passes tests on its own; branch
  history is preserved on merge, so each one stays readable forever.
- **Verify locally in preference to CI.** Hosted runners are slow; use them to
  confirm platform differences, not to discover ordinary mistakes.
- **If the same failure repeats, stop and report it.** Iterating on a wedged
  problem costs more than asking.
- **Fix what the work already touches; file what it does not.** A defect found
  beside the change gets fixed in the same pull request when three things hold:
  it is small, it is in the same theme as the work, and the tests already being
  run will prove it. Say in the reply that it was fixed and was not asked for,
  so it can be cut on review. Anything needing a refactor, a design decision,
  or an argument of its own goes to [`BACKLOG.md`](BACKLOG.md) in that same
  pull request — the moment it is noticed is the only moment its reasoning is
  cheap to write down. The option that is not available is noticing and doing
  neither.
- **Say plainly what is left for a human to do**, including anything that was
  deliberately left out.
- **Push back.** Honest recommendations are wanted, including for removing
  features or abandoning an approach. Agreement that isn't meant is worth
  nothing here.

## Keeping this current

These files carry the reasoning behind decisions, which is the part that is
expensive to reconstruct. When a decision changes, update the document in the
same pull request as the code — a standard nobody can trace back to its example
stops being persuasive, and stops being followed.
