# vump — design specification

This document is the authoritative specification for vump's behavior and
architecture. Where the source and this document disagree, treat the
disagreement as a defect in one of them and resolve it deliberately.

## 1. Purpose and scope

vump does two things:

1. **Bump** a semver version number, keeping every file in a repository that
   records that version in sync, and optionally turn the change into a git
   commit, tag, and push.
2. **Check** that a given version — typically a git tag in CI — matches the
   version recorded in source, failing loudly when it does not.

The second job is what makes the first one trustworthy. A tag that disagrees
with the source it claims to describe is a defect that must be caught before
any build, sign, or publish work is spent on it.

vump has no opinion about what a tag triggers downstream. Build matrices,
publishing to registries, and deployment are other tools' problems. vump's
responsibility ends at "the version in source and the tag agree, and both are
correct."

### Non-goals

- Running package-manager commands (`npm install`, `cargo build`, …).
  Resolving dependencies is somebody else's job, and doing it here would trade
  a fast, predictable edit for a slow one with network access, arbitrary
  install scripts, and its own failure modes.
- Deciding *when* to release, or orchestrating anything after the tag exists.
- Backward compatibility with any earlier configuration or CLI surface.

## 2. Domain model

### Version state machine

A version is either **stable** (`1.2.3`) or a **pre-release**
(`1.2.3-alpha.0`). Pre-release labels are ordered:

```
alpha < beta < rc
```

| Current         | Operation              | Result          |
| --------------- | ---------------------- | --------------- |
| `1.2.3`         | `patch`                | `1.2.4`         |
| `1.2.3`         | `minor`                | `1.3.0`         |
| `1.2.3`         | `major`                | `2.0.0`         |
| `1.2.3`         | `alpha --toward patch` | `1.2.4-alpha.0` |
| `1.2.3`         | `alpha --toward minor` | `1.3.0-alpha.0` |
| `1.2.3-alpha.0` | `alpha`                | `1.2.3-alpha.1` |
| `1.2.3-alpha.2` | `beta`                 | `1.2.3-beta.0`  |
| `1.2.3-beta.1`  | `rc`                   | `1.2.3-rc.0`    |
| `1.2.3-rc.1`    | `release`              | `1.2.3`         |

Rules that fall out of the table:

- Advancing to the **same** label increments its counter.
- Advancing to a **higher** label resets the counter to `0`.
- Moving to a **lower** label is refused. This is a mistake, not a workflow.
- Starting a pre-release from a stable version requires knowing which stable
  version it precedes, hence the mandatory `--toward`. It is spelled for the
  release being moved *to*, because `from` throughout vump means the version
  being left: `BumpPlan.from`, the channel pair in a backwards-move refusal,
  and the `from`/`to` keys in JSON output all point the other way.
- `release` on an already-stable version is an error.

### Deliberately excluded: `patch`/`minor`/`major` on a pre-release

Asking for `patch` while on `1.2.3-rc.1` is ambiguous: it could mean "finalize
this as `1.2.3`" or "abandon it and go to `1.2.4`". vump does not guess and
does not ask. It is an error, and the message directs the user to `release`
first, then bump from the resulting stable version. Two explicit steps beat one
command with a hidden fork.

### Version files

A version file is any file that records the project's version. Detection is by
filename, and the `MSBuild` project files are the only match on an extension
rather than a whole name — a .NET project is named after the assembly it
builds. The three extensions are one language each and one format.

| Filename            | Format     | Location of the version                 |
| ------------------- | ---------- | --------------------------------------- |
| `package.json`      | JSON       | `.version`                              |
| `*.csproj`, `*.fsproj`, `*.vbproj` | XML | `<Project><PropertyGroup><Version>` |
| `Directory.Build.props`, `Directory.Build.targets` | XML | `<Project><PropertyGroup><Version>` |
| `package-lock.json` | JSON       | `.version`, and `.packages[""].version` |
| `Cargo.toml`        | TOML       | `[package].version`                     |
| `Cargo.lock`        | TOML       | the sole `[[package]]` with no `source` |
| `pyproject.toml`    | TOML       | `[project].version`                     |
| `uv.lock`           | TOML       | each `[[package]]` with an editable `source` |
| `VERSION`           | plain text | entire file contents                    |

**Writes must preserve the rest of the file byte-for-byte.** Update the version
field in place rather than parsing and re-serializing: key order, indentation,
whitespace, and comments elsewhere in the file must survive untouched. A tool
that reformats a `package.json` as a side effect of bumping it will not be
tolerated in a repository, and correctness here is more important than the
elegance of the parsing approach.

A project file is full of other versions, and the one that moves is located by
element path rather than by searching for the word. `<PackageReference
Version="13.0.3" />` carries one as an attribute; `<VersionPrefix>` is a
different property vump does not split; `<AssemblyVersion>` and `<FileVersion>`
take four numeric parts and cannot hold a pre-release at all. Several
`<Version>` properties in conditional groups are alternatives, so which one
governs is MSBuild's answer to give and the file is refused rather than guessed
at.

`Directory.Build.props` and `Directory.Build.targets` hold settings shared by
every project beneath them. Both are authored and committed like a project
file, and neither is legacy. They differ in when MSBuild imports them: props
*before* the project body, so a project's own `<Version>` overrides it; targets
*after*, so it overrides every project. Either may be tracked, and the search
stops at the nearest one rather than merging. `Directory.Solution.props` is not
a version file — a `<Version>` there never reaches the projects.

A built assembly reports slightly more than vump wrote: inside a git repository
the SDK appends the current commit to the informational version
(`1.0.0+e559804…`), no SourceLink involved. That is semver build metadata added
after vump's job, and `check` compares tags to `<Version>`.

Support for arbitrary formats via a declarative extraction spec (a path or
pattern per file entry) is a plausible future direction, but the built-in
formats come first.

### Lock files

A lock file that records the project's own version is a version file by the
definition above, and is tracked like any other. `Cargo.lock`,
`package-lock.json` and `uv.lock` qualify: each records the version of the
project it locks, and `cargo build --locked`, `npm ci` and `uv sync --locked`
each reject a tree where a lock and its manifest disagree.

This does not weaken the non-goal. Resolving dependencies is a package
manager's job and vump never does it; restating a version vump has just written
is the same in-place edit it already performs on a manifest. The test for
whether an edit is allowed: **can the new contents be computed with no network
access and no knowledge of the dependency graph?**

`yarn.lock` and `pnpm-lock.yaml` record no version for the project itself — Yarn
pins its workspace at a placeholder precisely so a bump does not churn the file
— so they never go stale from a bump and are not tracked.

In `Cargo.lock` the entries that belong to the repository are the
`[[package]]` records carrying no `source`, since Cargo records one for
everything it fetched. A single-crate repository has one, and it needs no
naming.

`uv.lock` is the same shape with the marker inverted: uv records a `source`
for both, and the entries built from the tree are the ones marked `editable`.
That distinction is load-bearing rather than cosmetic — a published release of
a workspace member appears under the member's own name, and only the source
says which of the two the tree builds.

A workspace has one such entry per member, and the lock alone cannot say which
of them a project means — so the manifests declared alongside it do. Each
`Cargo.toml` or `pyproject.toml` names its package, and the entries a project
writes are exactly the ones its own manifests name. That covers both shapes a workspace takes:
members held at one version declare every manifest in a single project and
move together, while independently-versioned members declare one manifest each
and touch only their own entry. Members that are not declared are not touched.

Entries in scope must agree with each other. They describe one project moving
as a unit, so a disagreement between them is the same defect as a manifest
disagreeing with its lock, and is reported rather than resolved by preferring
one. Where nothing names a package — a project tracking a lock with no
manifest — the only inference left is that the repository builds exactly one,
and more than one is refused.

An npm lock records the version at the top level, and from `lockfileVersion` 2
onwards again under `packages[""]`. Both are written, because `npm ci` reads
the second. Locating them follows the key path rather than counting depth: a
dependency's version sits at the same depth as the project's own and differs
only by which key contains it. npm writes both records together, so finding
them already disagreeing means the file is corrupt, and it is refused rather
than settled by preferring one.

**A lock file that records the version but is not declared is refused before
anything is written**, naming the file to add. The search runs from the
manifest's own directory upward, because a workspace keeps one lock at its
root however deep the member sits. Writing the manifest alone would
produce a tag describing a tree that cannot be built from it — the exact defect
vump exists to prevent — and reporting it after the commit and tag already
exist makes recovery cost a tag deletion, a reset, and a redone release.

## 3. Configuration

A single `vump.toml`, discovered by walking upward from the working directory
and taking the first one found. Nothing bounds that walk and nothing about a
repository enters it: which configuration is in effect is settled by the files
themselves. TOML is the only supported format.

> **Rationale for TOML over YAML/JSON.** YAML's implicit typing is actively
> hazardous for a tool whose entire purpose is exact version strings: `1.0`
> becomes a float and loses its trailing zero, and bare `yes`/`no` become
> booleans. JSON has no comments, which the annotated default config depends
> on. Supporting several formats at once would also mean the same tool looks
> different in every repository, which is precisely the confusion this design
> avoids elsewhere.

### Single-project repository

The common case carries no ceremony:

```toml
files = ["Cargo.toml"]
```

### Multi-project repository

Independently-versioned projects in one repository are named, and addressed by
name:

```toml
[[project]]
name = "api"
files = ["services/api/Cargo.toml"]

[[project]]
name = "web"
files = ["apps/web/package.json"]
```

```sh
vump patch --project api
```

Naming rather than locating projects matters because the caller is frequently
not sitting in the project's directory — CI jobs and external automation invoke
vump from the repository root or from nowhere in particular. A model based on
per-directory config files cannot express "bump the project called `api`"
without first knowing where `api` lives.

`vump status` with no project selected reports every declared project and
whether its files agree.

**No command reports which `vump.toml` answered it.** With one configuration —
the only supported layout — there is nothing to report: the file is wherever
the upward search found it, and there was never a second candidate. With more
than one, the layout itself is the problem, and that is what the nesting
refusal names.

#### One configuration per repository

`[[project]]` is the answer to "this repository holds several things that
version separately." **A second `vump.toml` deeper in the same repository is
not a supported alternative; it is the mistake `[[project]]` exists to
prevent.** A repository shaped that way has given up every property this
design is built on:

- Projects can no longer be addressed by name, so `--project` does nothing and
  every bump requires standing in the right directory first.
- `vump status` can no longer report the repository, only whichever project the
  caller happens to be inside.
- A pushed tag can no longer be traced back to a project, because the pattern
  that would identify it lives in a file the verifier never reads.
- Git operations run against the enclosing repository regardless, so a nested
  configuration commits and tags into a repository it does not describe.

The last of those is not theoretical. A bump run from a nested configuration in
this repository's own `sandbox/` produced a `v1.0.1` commit and tag against
vump itself, from a project that has nothing to do with vump's version.

The sandbox is the one place nested configurations are kept deliberately, and
only because those projects exist to be run against by hand. They are the
exception that shows the cost: each has to name its own tag and commit so that
neither can be mistaken for a release of vump.

**Acting on a nested configuration is refused**, naming both files and pointing
at `[[project]]`. Both are named in full: a path relative to the other starts
partway down the tree whenever a third configuration is involved, and reads as
if it started at the top. An absolute path needs no frame, so there is no frame
for a reader to guess at. Nesting is decided by continuing the discovery walk one directory higher: if
another `vump.toml` lies above, the one in effect is shadowing it. Being the
same walk, the check cannot disagree with the resolution it is checking, and it
needs no notion of a repository — a directory holding its own `.git` makes no
difference. Refused rather than warned, for the reason a release publishing no
checksums is: by the time a warning is printed about a pushed tag, the tag is
on the remote.

`--allow-nested` proceeds anyway. It is a flag rather than a setting in the
nested file deliberately. A setting would answer the question once and stay
answered, pre-approving every later run from that directory including the
accidental one, and nobody re-reads a `vump.toml` before typing `vump`. The flag
asks again every time, which is what a person who has forgotten where they are
standing needs. Its inconvenience is the point rather than a cost: it is a
standing reminder that `[[project]]` is going unused, and the way to stop typing
it is to restructure the configuration rather than to keep typing it.

It is not a preference with a default worth overriding, which is why it has no
matching setting — the same shape as `vump init --force`.

**Every command touching a project is refused, reading included.** The hazard
is not writing but operating on the wrong configuration at all, and `check` is
the sharpest case rather than an exception to it: it answers whether a version
matches, so a nested project whose version happens to coincide answers *yes*
about the wrong project — a confident false pass, from the one command whose
whole purpose is catching a version that lies. A refused write is a mistake
someone notices; a wrong verdict is one they believe.

`init` is refused for the opposite reason: writing a configuration beneath one
that already exists is where the arrangement every other command refuses comes
into being.

Only `self` commands are exempt, and only because they act on the installed
binary and never load a configuration at all.

That leaves one rule with no carve-outs to remember, and one gate: the check
runs where configuration becomes known, before any command is dispatched, so a
command added later cannot be written without it.

### Tag patterns

`tag_pattern` may be set per project, overriding the repository-wide one, and
both it and `commit_message` accept a `{project}` placeholder so that one
repository-wide template can cover every project. `tag_message` accepts the
same placeholders.

This is not cosmetic. Independently-versioned projects sharing one pattern
collide as soon as two of them reach the same version, and a pushed tag carries
no indication of which project it describes — which makes tag verification, the
thing vump exists for, impossible in exactly the repositories `[[project]]` was
added to serve.

A pattern is read in both directions: it names the tag a bump creates, and it
recognizes which project a pushed tag belongs to and what version it claims.
That is what lets CI pass a tag through without stating the project.

Matching is literal — a fixed prefix, the version, a fixed suffix — rather than
a regular expression, so a pattern cannot match more than it appears to.

When several projects claim one tag, that is reported rather than guessed at: a
tag attributed to the wrong project would verify the wrong source. An argument
matching no pattern is read as a bare version, which keeps `vump check 1.2.3`
working in a single-project repository.

### Git integration

```toml
[git]
through = "tag"                  # or "none", "commit", "push"
commit_message = "chore: bump version to v{new_version}"
tag_pattern = "v{new_version}"
tag_style = "annotated"          # or "lightweight", or "signed"
tag_message = "Release {new_version}"
release_branches = ["main"]      # optional; unset means any branch
prerelease_branches = ["main"]   # optional; unset means any branch
```

**How far a bump goes is one ordered setting, not several booleans.** The steps
are cumulative — each performs everything the one before it does, the way a log
level includes the levels beneath it — so `through = "tag"` also commits.

Naming only the furthest step is what makes a combination like "tag without a
commit" impossible to write. Three independent booleans can spell eight states
of which half are contradictory, and a tool that accepts them has to detect and
silently repair the rest; a tool that cannot represent them has nothing to
repair. It also means one setting is overridden by one flag, in either
direction, rather than a flag per boolean plus an escape hatch.

An absent `through` is **not** a fourth step. Nothing has been decided, so an
interactive run asks and a subcommand does no git work. `through = "none"` is
the opposite — a decision that this repository never touches git — and is acted
upon without asking. A repository that keeps its projects inside another one's
checkout needs to be able to say that, and needs to be believed.

`vump init` writes `through = "tag"`. That matches what most version tools do
by default (`npm version` commits and tags, and leaves pushing to you), and it
stops one step short of the only action that reaches other people.

Tags are **annotated by default**. A lightweight tag is a bare pointer with no
tagger, date, or message; `git describe` prefers annotated tags and some
release tooling ignores lightweight ones outright. A release tag is precisely
what the annotated format exists for, so producing the weaker kind by default
would be a poor decision made silently. `tag_style = "lightweight"` restores
it.

`signed` produces an annotated tag, signed. A signature is a property of an
annotation rather than an independent setting: there is nothing to sign
without a message, and the configuration cannot express the combination. Where
git prompts for a key it prompts, exactly as it already does for `git commit`
under `commit.gpgsign` — that is git's business, not a prompt vump introduces.

**Configuration is authoritative.** If `vump.toml` says `through = "tag"`, vump
commits and tags — it does not ask, in any mode. Settings present in
configuration are decisions the user has already made; re-asking them is a
defect.

**A flag replaces the setting it names, for one run.** Configuration sets the
default; a flag overrides it; both use the same name and the same values. The
override reads in both directions, so a configured step can be lowered as
readily as raised — `--through none` when a push is configured but the result
wants checking first, `--through push` when it does not.

A setting earns a flag when someone could reasonably want a different answer
for one run without changing the repository's convention: `--through` and
`--tag-style` qualify, the latter for a signing key that is temporarily out of
reach. `tag_pattern` deliberately does not. `check` reads it in reverse to work
out which project a pushed tag belongs to, so a one-run override would produce
a tag that verification cannot attribute — the pattern is a parser, not a
preference.

#### Where a release may be tagged from

`release_branches` and `prerelease_branches` name the branches a release may be
tagged from. Both are optional and independent, and **absence means
unconstrained** — an empty list is refused when the file is read, because it
could only ever refuse and the run that reads it is never the run that can act
on the answer.

**Which list applies is decided by the version, not by the command.** A target
carrying a pre-release component answers to `prerelease_branches`; one without
answers to `release_branches`. So `set 1.2.3` is governed exactly as `patch` is
and `set 1.2.3-beta.1` exactly as `beta` is, and nothing has to enumerate which
commands release.

**They are separate settings because the two are held to different standards
almost everywhere.** A stable tag is what people install, and every release
tool agrees it comes from the trunk. A pre-release is how unfinished work is
shared *without* merging it first — GitVersion derives a pre-release label from
the feature branch as its core model, and semantic-release ships prerelease
branches by default. Constraining both with one list would mean merging before
sharing, which is the thing a pre-release exists to avoid.

The reason to make it configurable at all rather than simply exempting
pre-releases: the tools that publish freely from feature branches mostly do not
write a durable git tag — GitVersion computes a version without tagging, and a
changesets snapshot is a dist-tag publish. vump writes a real tag, which
outlives the branch. So the practice is mainstream but not free here, and a
team that would rather not pay for it can say so.

**Only a run that reaches a commit is covered.** A run that writes files and
stops leaves nothing behind that could belong to the wrong branch, and refusing
it would block bumping a version inside a pull request — which is where a
version is supposed to change. This is narrower than the nesting refusal
deliberately: nesting refuses reading too, because `check` can pass confidently
about the wrong project, and reading on a feature branch is not wrong in that
way.

The check runs once the plan is composed and before it is applied, so the target
version is known and a refusal has still written nothing. A `--dry-run` is
refused as well, on the same reasoning as a nested one: a plan for a run that
would be refused is not a plan.

**A detached `HEAD` is refused where a policy exists**, since a tag made there
belongs to no branch at all. Where no policy exists it is nobody's business but
the caller's — the check is opt-in in both directions.

**The guided run marks rather than refuses.** A bump the policy excludes is
listed and marked, not dropped: an option that is simply missing is something
to work out, whereas one shown as blocked explains itself. Choosing it asks
again rather than proceeding. Where nothing at all is selectable there is no
menu to show, so the refusal is raised directly. `--any-branch` brings the same
bumps back marked — the flag restores the choice, it does not make it look
safe. Every one of those answers comes from the same `check_branch` a
non-interactive run uses, so the menu and the refusal cannot disagree, and the
questioner is not trusted to honour the marking: a blocked bump returned anyway
is refused by the rule.

`--any-branch` proceeds anyway, and the run says what it waived. That notice is
not the one [`BACKLOG.md`](BACKLOG.md) decided against: that entry rejected
announcing which *source* a setting came from, a notice with nothing to do
about it. This one reports a hazard the caller can still act on, in the window
where acting is cheap, and it fires only when the flag actually overrides
something.

## 4. CLI contract

The single most important property of this CLI: **whether an invocation is
interactive is determined by exactly one thing — the presence of a
subcommand — and there are no exceptions.**

| Invocation                   | Behavior                                     |
| ---------------------------- | -------------------------------------------- |
| `vump`                       | Interactive. Always.                          |
| `vump patch` (and siblings)  | Non-interactive. Never prompts, under any circumstances. |

A user must never have to wonder whether a command is about to block on a
prompt. An invocation that would need information it does not have **fails with
an actionable error** rather than falling back to asking:

- **Files disagree** on the current version → error listing the disagreement.
  There is no flag to resolve this non-interactively; a source of truth
  contradicting itself is an exceptional state that a human should look at.
- **Starting a pre-release from stable without `--toward`** → error. `--toward`
  is required, not optional-with-a-prompt.
- **`patch`/`minor`/`major` on a pre-release** → error, per §2.
- **An undeclared lock file that records the version** → error naming it.

More generally: everything knowable before writing is checked before writing,
so a run that cannot produce a consistent result performs none of it and leaves
no partial state to unwind. Where partial completion is genuinely unavoidable —
a push failing after the commit and tag succeeded — exactly what did happen is
reported instead.

In interactive mode, the only question vump asks unconditionally is the bump
type, and its menu lists **only operations the current version permits** — a
transition that could not apply, such as `patch` on a pre-release, is omitted
rather than displayed with an explanation of why it will fail. A transition
that would apply but the branch policy excludes is a different case, and is
marked rather than dropped, per §3. Everything else is asked only when
configuration has not already settled it.

Every label the menu prints is the non-interactive invocation for that choice,
so the menu doubles as documentation of the flag it teaches.

There is deliberately **no `-y`/`--auto-approve` flag.** Once a subcommand
never prompts by construction, the subcommand *is* the confirmation, in the
same way `rm file` needs no confirmation flag.

### Commands

```
vump                      Interactive bump
vump patch|minor|major    Bump a stable version
vump alpha|beta|rc        Start or advance a pre-release (--toward required from stable)
vump release              Drop the pre-release suffix
vump set <version>        Write an exact version to every tracked file
vump check <version>      Verify tracked files match the given version
vump status               Report current versions and sync state
vump init                 Create a vump.toml
vump self update          Install a published release
vump self status          Report whether a newer release exists
vump self list            List published releases
```

### Flags

| Flag                | Applies to      | Description                                    |
| ------------------- | --------------- | ---------------------------------------------- |
| `--dry-run`         | bump commands   | Compute and report the plan, write nothing     |
| `--toward <bump>`   | pre-release     | Stable release the pre-release leads to        |
| `--project <name>`  | all             | Select a project in a multi-project repository |
| `--through <step>`  | bump commands   | How far to carry the release: `none`, `commit`, `tag`, `push` |
| `--tag-style <style>` | bump commands | How the tag object is written                  |
| `--allow-nested`    | global          | Act on a configuration nested below another    |
| `--any-branch`      | global          | Release from a branch the configuration does not list |
| `--json`            | global          | Machine-readable output                        |
| `--channel <name>`  | `self`          | Least mature kind of release to accept         |
| `--to <version>`    | `self update`   | Install this exact version, newer or older     |
| `--force`           | `init`          | Overwrite an existing configuration            |
| `--limit <n>`       | `self list`     | Show at most this many releases, newest first  |

Both replace the `[git]` setting of the same name for one run. There is no
escape hatch to keep separate from them, because `--through none` *is* the
escape hatch, and no pairing of flags can contradict itself when only one names
the answer.

A flag name is scoped to the commands it applies to, so the same spelling may
mean different things in different columns above and never appear together in
one invocation. `--to` takes a version when updating the binary; a bump
command's `--toward` takes a bump name. Reading a name as reserved across the
whole surface is a mistake — the "Applies to" column is what settles it.

Only files declared in configuration are staged. If the run reaches a commit, a
dirty working tree is a hard failure — a version bump must not sweep unrelated
changes into its commit.

If push fails after a successful commit or tag, vump reports exactly what did
succeed and prints the command to finish manually. Partial success must never
be reported as total failure.

### Setting an exact version

A bump refuses tracked files that disagree, and offers no flag to resolve it
non-interactively. `set` is the deliberate counterpart: it writes an exact
version to every tracked file and requires no prior agreement, because
disagreement is what it repairs.

It does not refuse to move backwards. The reasoning matches `self update --to`:
a version written out by hand is consent, and refusing would leave no way to
undo a mistaken bump.

`set` produces no single "current" version at all. When the files disagree
there is not one, and reporting the first would misdescribe the rest, so each
file's own previous version travels in the per-file changes. A shared version,
where the files happen to agree, is derived from those rather than stored
beside them.

Setting the version the files already record is reported as success with
nothing done. Git refuses an empty commit, so a run that would produce no
change stops before reaching it rather than failing for a reason unrelated to
what was asked.

## 5. Output and exit codes

Human-readable output is the default. `--json` selects machine-readable output
and is a presentation concern only: both renderings are produced from the same
structured result, and neither may carry information the other lacks.

`--json` requires a non-interactive invocation. Combined with bare `vump`, it
is an error rather than an attempt to interleave prompts with parseable output.

JSON output covers every command, not only `check`. For bump commands it
reports each file changed with its old and new version, the commit created, the
tag name, and whether the push succeeded — values that come into existence
during the run and that a caller would otherwise have to parse out of prose.

Decorative output (colour, symbols) is suppressed automatically when stdout is
not a terminal.

Exit codes are part of the public contract. Each distinct failure mode gets its
own stable code so callers can branch without matching strings:

| Code | Meaning                                        |
| ---- | ---------------------------------------------- |
| 0    | Success                                        |
| 1    | Generic / unexpected error, a declined prompt, or `self status` finding an update |
| 2    | Usage error (bad arguments)                    |
| 3    | Configuration missing or invalid               |
| 4    | Version mismatch (`check` failed)              |
| 5    | Tracked files disagree with each other         |
| 6    | Working tree dirty                             |
| 7    | Invalid version transition                     |
| 8    | Git operation failed                           |
| 9    | A release artifact could not be trusted        |
| 10   | A release could not be obtained                |

9 and 10 separate the two ways `self update` can fail to install. 9 means the
downloaded artifact does not match its published checksums, or the release
publishes none — that warrants looking into, since it means tampering or
corruption. 10 means the release list was unreachable, the requested version
does not exist, or this platform has no published binary for it — retrying, or
naming a different version, may well succeed. Collapsing them into one generic
failure would hide which reaction is the right one.

`self status` deliberately reuses code 1 rather than introducing one of its
own: a script that only wants to know whether to act does not need a third
category between "nothing to do" and "something went wrong", and the human
output already says which of the two happened.

## 6. Architecture

Ports and adapters, sized to the problem: one crate, clear internal
boundaries. Splitting into multiple crates is premature until something is
demonstrably reusable outside this binary.

```
src/
  domain/     Pure logic. No I/O, no clock, no environment.
              Version state machine, bump planning, version file formats.
  config.rs   Configuration as plain data: parsing and validation.
              No I/O — a test builds the value it wants directly.
  ports.rs    Traits describing what use cases need from the world.
              FileSystem, Vcs, Interaction, ReleaseSource.
  adapters/   Concrete implementations of the ports.
              Real filesystem, git via subprocess, terminal prompts,
              GitHub releases, and in-memory doubles for tests.
  app/        Use cases wiring ports together: Bump, Check, Status, Init,
              Update, the guided run, and locating configuration. The layer
              worth testing hardest.
  cli/        clap definitions, adapter selection, output rendering.
```

Rules that keep the boundaries real:

- `domain` depends on nothing but the standard library and pure helpers. If it
  needs to read a file or know the time, the design is wrong.
- `app` depends on `domain` and `ports`, never on `adapters`. Use cases are
  constructed with port implementations supplied by `cli`.
- **Only `adapters` names `std::fs`.** Everything above it reaches the
  filesystem through the `FileSystem` port, discovery included — finding
  `vump.toml` is a use case (`app::discover`) rather than something
  configuration does for itself. That is what lets discovery be exercised
  against an in-memory tree, and it keeps `config.rs` free of the I/O its own
  documentation says it has none of.
- **The guided run is a use case, not a CLI detail.** It carries more
  decisions than any other path and is the one most runs take, so it lives in
  `app::guided` and reaches the terminal only through ports. Presentation does
  not follow it down: the confirmation text and the name of each bump are
  passed in as functions, so the use case decides what to ask and `cli` decides
  how it reads. `Interaction::notice` exists for the same reason — a hazard
  reported mid-run goes through the port rather than being printed from
  underneath the abstraction.
- A single `ChangeSet` value is the input to the human renderer, the JSON
  renderer, and `--dry-run` alike. Those three must not compute anything
  themselves. Bumping and setting differ only in how they reach a target
  version; what they produce is the same type, so neither has to describe
  itself in the other's terms.

### Adapter notes

- **git** is invoked as a subprocess rather than through a library. This
  inherits the user's real git configuration, credential helpers, hooks, and
  SSH setup for free, and keeps the dependency surface small.
- **Interaction** has three implementations: a terminal one, one that returns
  an error for every question, and a scripted one for tests. Non-interactive
  mode is enforced by construction — it is given the erroring implementation,
  so a prompt cannot leak into a scripted run even by accident.
- **Self-update** replaces the running binary. On Unix an atomic rename works
  even while executing; on Windows the running executable must be renamed aside
  first, then replaced. That platform difference is delegated to a library
  rather than reimplemented. Downgrades are refused: a development build ahead
  of the last release must never be overwritten by it.
- **Downloaded artifacts are verified.** Every release publishes a `SHA256SUMS`
  asset; self-update and the CI action both check what arrived against it
  before writing to disk or executing. The digests are computed *after*
  signing, since signing rewrites the macOS binary and a digest taken earlier
  would describe an artifact nobody receives.

  A release publishing no checksums is refused, not installed with a warning.
  The cost — releases predating checksums cannot be installed by `self update`
  — is accepted; the reasoning is [`ENGINEERING.md`](ENGINEERING.md) §3.

  Read-only commands (`self status`, `self list`) download nothing and so
  require nothing to verify.

- **Release provenance is attested.** Checksums prove an artifact arrived
  intact; they cannot prove it was built here, since anyone able to upload to a
  release could publish a matching listing beside a binary of their choosing.
  Each release therefore also carries a SLSA provenance attestation, signed
  with a short-lived certificate derived from the release workflow's own OIDC
  identity and recorded in a public transparency log. An artifact uploaded out
  of band cannot carry a valid one, and there is no long-lived key to store,
  rotate or leak.

  Anyone can verify one — `gh attestation verify <file> --repo codehacks-io/vump` —
  but the tooling enforces checksums instead, for the portability reason in
  [`BACKLOG.md`](BACKLOG.md).

- **Release discovery** reads the full release list rather than the endpoint
  that returns only the latest non-pre-release, because that endpoint answers
  one channel's question. Maturity is derived from each version itself, so it
  cannot disagree with how a release happened to be flagged when published.

### Release channels

`--channel` names the *least mature* release that may be installed, defaulting
to stable. It is a floor rather than an exact match, and the distinction
matters: semver compares the version core before the pre-release, so
`1.1.0-alpha.0` outranks `1.0.0-rc.5`. Taking simply the newest pre-release
would move someone tracking release candidates onto the next minor's first
alpha — a version upgrade but a stability downgrade.

A pre-release whose label vump does not recognize is never offered on any
channel: without a rank there is no way to judge it against a floor.

The floor is per-invocation and deliberately not remembered. Persisting it
would require installation-level state, which the tool otherwise has no need
for, and one setting does not justify introducing it.

`--to <version>` installs exactly the version named, bypassing both the channel
and the refusal to downgrade. That is what makes a rollback expressible, and it
is safe to allow because the caller had to write the version out.

### Dependency notes

Prefer `rustls` over any OpenSSL-backed TLS stack for anything performing
network I/O: release binaries are statically linked against musl, and native
TLS does not cooperate with that.

## 7. Testing

Three layers, each with a distinct job:

1. **Domain unit tests.** Every transition in the §2 table, plus the refused
   ones. Pure functions, no fixtures, exhaustive.
2. **Use-case tests** against in-memory port implementations. A fake VCS, a
   fake filesystem and a scripted `Interaction` let the whole bump flow be
   exercised — including git side-effects, failure paths, and the guided run —
   without a real repository or a terminal. `MemoryInteraction` answers from a
   script and records every question, so what a run *asked* is assertable as
   well as what it decided: a question the guided path should have answered for
   itself is a defect a test can see.
3. **End-to-end tests** driving the compiled binary against temporary
   directories, asserting on stdout, stderr, and exit codes. This layer owns
   the §4 interactivity contract and the §5 exit-code table: it is the only
   place that can prove a subcommand never prompts.

Formatting preservation deserves explicit tests: bump a file with unusual
indentation, key order, and comments, and assert that only the version bytes
changed.

## 8. Release engineering

Release assets are named `vump-<os>-<arch>` (with `.exe` on Windows), plus
`vump-darwin-universal`. These names are a contract: the CI check action and
in-place self-update both resolve them.

- Linux binaries link statically against musl so they run on any distribution
  regardless of glibc version.
- macOS ships a single universal binary, which is the only macOS artifact
  signed and notarized. Bare Mach-O executables cannot be stapled; Gatekeeper
  verifies online on first launch.
- The release workflow verifies the tag against source *before* building
  anything.
