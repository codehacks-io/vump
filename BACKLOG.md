# Backlog

Work that is not built yet, and decisions taken against building things.

[`DESIGN.md`](DESIGN.md) describes how vump behaves *now* and is authoritative
for the code. This file is the opposite: it holds what is not settled. When an
item here is built, its rules move into `DESIGN.md` and the entry is deleted.

**Decided against** entries exist so a question is not reopened without new
information. If you find yourself proposing one of them, say what changed.

**A fix small enough to make now does not come here.** This file holds work
that needs a decision, a design or a refactor. It is not a holding pen for
things that could have been done in the pull request that found them — an entry
reading "rename this" or "this comment is stale" costs more to file and re-read
than to fix.

---

## Ready to build

Empty. Work arrives as an idea below and moves here once its problem is
agreed, at which point it is ranked by value.

## Ideas, not yet decided

Each says what would settle it. Nothing moves up until something does.

### Declarative version-file formats

Detection is by filename. Anything else — `gradle.properties`, a `*.gemspec`, a
version in a shell script — would need a per-entry extraction spec: a path for
structured formats, a pattern for the rest.

Weaker than it looked. Both formats this entry once named as motivation,
`pyproject.toml` and `*.csproj`, were added as built-ins instead, neither
costing more than a day. A spec would change the configuration schema
permanently to avoid work that keeps turning out to be small.

What would settle it: a format someone needs that is not worth adding built-in.

### A Rust project in the sandbox

The sandbox covers npm and C#. Rust is missing, and the argument at the time
was that this repository is itself the Cargo example — it tracks `Cargo.toml`
and `Cargo.lock` and is exercised on every release.

That argument is weaker than it looked. The repository demonstrates a
single-crate project only, so the shape actually worth showing by hand — a
workspace, where the lock holds one entry per member and a project writes only
the entries its own manifests name — has no worked example anywhere outside the
test fixtures.

The reason not to rush: a crate inside this repository's tree is not inert the
way an npm or C# project is. It would need excluding from the workspace, and a
mistake there breaks `cargo build` for the tool itself.

### Inputs on the check action

The composite action takes `version`, `config` and `vump-version`. Passing a
tag now selects its own project, so a `project` input is only needed for a
repository that verifies bare versions rather than tags.

There is no `allow-nested` input either, so a repository whose `config` input
points at a nested `vump.toml` now fails in CI. That is arguably the right
outcome — it surfaces the layout rather than verifying quietly against the
wrong project — but it is a consequence that was not chosen deliberately, and
the alternative is one input.

### Pushing what a bump created, separately

A bump that stops at `through = "tag"` leaves a commit and a tag to push by
hand. `git push --follow-tags` does it, but pushes *every* annotated tag
reachable — in a monorepo that can publish another project's tag that happened
to be sitting unpushed. vump's own push names the single ref it created, which
is strictly narrower.

A `vump push` would close that gap: read the current version, render the
project's tag pattern, push `HEAD` and that one tag. It needs no stored state,
since both are derivable from the files and the pattern.

Undecided because the repository that would feel it — a monorepo with several
projects tagged independently — does not exist here yet, and the single-project
case is served by `git push --follow-tags` today.

Not to be confused with a prompt between tagging and pushing, which was
considered and rejected: a blocking prompt owns the terminal, so it cannot
deliver the inspection it appears to offer. Stopping at `tag` and looking
around with a free shell is strictly better.

### Deciding the bump from commit messages

semantic-release and release-please read Conventional Commits — `feat:`, `fix:`,
`BREAKING CHANGE:` — to work out whether a release is a patch, a minor or a
major, so nobody names the number. It is the most widespread thing in release
tooling that vump does not do, and its absence here is currently an omission
rather than a decision.

What it would give: a release that needs no judgement at the moment it happens,
which is what makes a fully automated release from CI possible at all.

What it costs: the number becomes a function of commit discipline. A `fix:` that
was really a breaking change ships as a patch, and nothing downstream catches
it — a confident wrong answer, which is the exact failure `vump check` exists to
prevent in the adjacent case. It is also all-or-nothing per repository, since
one unconventional commit silently drops out of the calculation.

If it is built, it should propose rather than decide: compute the bump, then
require it to be confirmed or overridden, so the number stays a decision while
the work of reaching it goes away. That also keeps the interactive run and the
subcommands telling the same story.

What would settle it: whether anyone wants to release a vump-managed project
from CI with no human in the loop. Nobody has asked yet.

### Generating a changelog

standard-version, release-please, changesets and semantic-release all write
`CHANGELOG.md` as part of the release. vump does not, and it is the most likely
thing to be asked for.

The argument for: the changelog describes exactly the version being tagged, so
it belongs in the same commit. Written separately it drifts from the tag it
describes, which is the same class of defect vump exists to catch.

The argument against: it needs a source for the entries, and every source is a
commitment vump has so far avoided. Commit messages require Conventional
Commits, and so inherit that entry's problem. Hand-written fragments require
changesets' whole parallel workflow. Pull request titles require a forge API,
which drags vump into knowing about GitHub for something that is not
verification.

Note that this is not excluded by the non-goals: a changelog is neither deciding
when to release nor orchestrating anything after the tag. It sits inside the
window vump already owns, which is why the question is open rather than closed.

What would settle it: finding a source of entries that needs no new workflow
and no forge.

## Decided against

### A `-y` / `--auto-approve` flag

Naming a subcommand already means vump never prompts, so the subcommand *is*
the confirmation — the same reason `rm file` needs no confirmation flag.

### Configuration in YAML or JSON

YAML's implicit typing is hazardous for a tool whose subject is exact version
strings: `1.0` becomes a float and loses its trailing zero. JSON has no
comments, which the generated configuration relies on. Several formats would
also make the same tool look different in every repository.

### Running the package manager to refresh a lock file

vump writes a lock file's own version entry, which needs no network and no
knowledge of the dependency graph. Shelling out to `npm install` or `cargo
check` instead would mean unbounded runtime, network access, arbitrary npm
lifecycle scripts, and guessing which package manager a repository uses.

The line: vump may write a value it already computed; it may never resolve
dependencies.

### Deriving the version from git tags instead of files

Tools like MinVer compute the version from the nearest git tag, so no file
records it. It needs git history at build time, which shallow CI checkouts and
source tarballs lack; it leaves a checked-out tree with no readable version;
and it does not reach npm or Cargo, whose manifests carry one regardless. Most
of all it removes the second opinion — a mistyped tag becomes a correctly-built
wrong version, the failure `vump check` exists to catch.

`vump set "$(git describe --tags --abbrev=0)"` already covers that direction
for anyone who wants it.

### A command to resume an interrupted bump

The failure it would recover from no longer happens: everything knowable before
writing is checked before writing, so a run that cannot finish cleanly does
nothing at all. It would also need persisted state, rejected for `--channel` on
the same grounds. `vump set <version>` is the repair.

### Tracking yarn.lock or pnpm-lock.yaml

Neither records the project's own version, so a bump cannot make either stale.
Measured rather than assumed: with `"version": "1.2.3"` in `package.json`, a
generated `yarn.lock` and `pnpm-lock.yaml` contain zero occurrences of it.

### Reporting which `vump.toml` answered

Shipped in 0.4.0, removed in 0.5.0. With the one supported layout it prints
`vump.toml`, a line carrying nothing; it says something only when
configurations are stacked, which is the arrangement `[[project]]` exists to
prevent. Every frame for naming it was wrong somewhere — relative to the
configuration above, three deep, it printed a fragment anchored to a file the
reader cannot see.

What closes it: the nesting refusal already names both files in full, and fires
exactly when the answer matters.

### Announcing that a flag overrode configuration

Output reports the effective plan, never where each decision came from. The
flag is in the command the caller just typed, the result already names what
happened, and `--dry-run` exists for asking in advance. A notice with no action
attached fails the same test that governs error messages.

### Enforcing provenance verification, not just publishing it

Releases carry a signed attestation and anyone can check it with `gh
attestation verify`; the action and self-update enforce checksums instead.
Portability, not doubt — checksums need only `curl` and a hash utility, while
verifying an attestation needs a recent `gh` or a Rust Sigstore implementation,
and those crates are immature. Revisit if that changes.

### Per-identifier release branches

Splitting branch policy per channel — `rc` from `release/*`, `beta` from
`develop` — rather than once at stable versus pre-release. It triples the
configuration surface for a policy rare even in GitFlow shops, and the existing
split covers the three arrangements people actually hold. Reopen if someone
names a repository needing the third axis.

### Branch patterns in the release-branch lists

Exact names cover `main`, `master` and `release`, which is what nearly every
repository needs, and a pattern language is far easier to add than to narrow
once written. semantic-release supports globs and regex in this position; the
regex half is the part worth not copying.

### Remembering an update channel

`--channel` is per-invocation. Persisting it needs installation-level state — a
config directory vump otherwise has no need for — which one setting does not
justify.

### Per-project commit messages

`commit_message` accepts `{project}`, which distinguishes a monorepo's commits.
Overriding the whole message per project, as `tag_pattern` allows, has no
motivating case: tags must be unique, commit messages need not be.

### A saved plan-then-apply workflow

Terraform's plan/apply exists because infrastructure changes are slow,
expensive, and reviewed by someone other than their author. Bumping a version
is none of those, and `--dry-run --json` already emits the plan. Revisit if a
plan is ever reviewed asynchronously by someone who did not produce it.
