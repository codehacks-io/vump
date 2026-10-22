# How work is done here

Process rather than code style: how a change gets from a branch to a release.
For how the code itself is written and judged, see
[`ENGINEERING.md`](ENGINEERING.md).

## The local loop

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs exactly this, on Linux, macOS and Windows. Run it locally before
pushing — hosted runners are slow, and discovering a formatting error on one
costs minutes for something that costs seconds here. Use CI to confirm genuine
platform differences, not ordinary mistakes.

## Branches and pull requests

- Branch from `main`, named for the change (`set-version`,
  `release-checksums`).
- **Open a pull request when a coherent set of work is done, then pause for
  review.** A pull request is a checkpoint, not a formality: it is where review
  catches what tests cannot.
- The pull request description explains what changed and why it is shaped that
  way. If the shape changes during review, update the description — a
  description that no longer matches its own branch misleads whoever reads it
  next.

## Commits

- **Small and frequent.** Every commit compiles and passes tests on its own.
- Merges preserve full branch history rather than squashing, so every commit
  message is permanent. Write them for someone reading in a year.
- Conventional prefixes: `feat`, `fix`, `refactor`, `chore`, `ci`, `docs`,
  `test`.
- The subject is imperative and specific. The body says *why*, not what — the
  diff already says what.

## Releases

vump versions itself with vump, as two projects that ship independently.

```bash
vump patch --project main --through push        # or: alpha, beta, rc, release
```

| Project | Tracks | Tagged | Ships |
| --- | --- | --- | --- |
| `main` | `Cargo.toml`, `Cargo.lock` | `v1.2.3` | The binary: build matrix, signing, checksums, attestation, release |
| `website` | `website/package.json` | `website-v1.2.3` | [`website/`](website) to GitHub Pages |

The tag shape decides which workflow runs, and `vump check` infers the project
from it, so neither has to be told `--project`. A copy fix on the landing page
therefore ships without a vump version — cutting one to deploy the site would
tell everyone pinning the binary that something changed when nothing did. The
same separation costs the reverse: a binary release does not redeploy the site,
so the version the page states lags until the site is released too. See
[`website/README.md`](website/README.md).

The rest of this section is about the `main` project. The website's number
answers a narrower question — which build of the page is deployed — and needs
none of the reasoning below, since nothing resolves it.

### Choosing the number, while the major is 0

**Until 1.0, the minor is the breaking slot.** This is not a formality: Cargo
and npm both resolve `0.3.1` as compatible with `0.3.0` and refuse `0.4.0`, so
the number is the only signal anyone pinning a version receives.

| Bump | For |
| --- | --- |
| Minor (`0.3.0` → `0.4.0`) | Anything that breaks a working setup: a configuration key renamed or removed, a flag removed, or an invocation that used to succeed and now fails |
| Patch (`0.3.0` → `0.3.1`) | Everything else |

The test is whether an existing setup stops working — **not** whether the old
behavior deserved to keep working. A refusal added for good reasons still
breaks whoever relied on the thing now refused, and shipping that as a patch
tells every resolver that nothing changed. vump exists to stop a version number
from lying about its source; its own numbers are held to that.

Reaching 1.0 is what changes this: the major becomes the breaking slot and the
minor goes back to meaning additive.

A change with nothing in it for someone running the binary — documentation, a
test, repository configuration — needs no release of its own and rides along
with the next one that does.

Pushing the tag runs the release workflow, in this order:

1. **Verify** — the tag is checked against the version in source before
   anything is built, then format, lint and tests run. A tag that lies costs
   nothing.
2. **Build** — six targets, including static musl Linux binaries and a Windows
   arm64 cross-compile.
3. **Sign** — the macOS universal binary is signed and notarized.
4. **Checksum** — `SHA256SUMS` is produced *after* signing, because signing
   rewrites the binary and a digest taken earlier would describe an artifact
   nobody receives.
5. **Attest** — keyless SLSA provenance, signed with a short-lived certificate
   from the workflow's own identity and recorded in a public transparency log.
   There is no key to store, rotate or leak.
6. **Publish** — the release and its assets.

**Cut an alpha whenever a change is worth trying on a real machine.** Something
that passes tests and looks finished is not the same as something used once in
anger, and an alpha costs one tag.

Asset names (`vump-<os>-<arch>`, plus `vump-darwin-universal`) are a contract:
the CI check action and in-place self-update both resolve them.

## Where things get written down

Keeping the documents true is part of the change, not follow-up work: whichever
document [`CLAUDE.md`](CLAUDE.md) says holds a thing is updated in the same
pull request that changes it. How `DESIGN.md` and `BACKLOG.md` divide between
them is stated in [`BACKLOG.md`](BACKLOG.md), where it is needed.

The sandbox projects share this repository, so every `vump.toml` in them keeps
git off: a commit or tag made there lands here. Git behavior is covered by the
end-to-end suite, which builds a throwaway repository per test.
