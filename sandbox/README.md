# Sandbox

Working projects to run vump against by hand, one per ecosystem and per shape.
They are real: the npm lock files came from `npm install`, the C# projects from
`dotnet new`, and each prints its own version when run. Reading them shows what
a `vump.toml` looks like for that ecosystem; running vump in them shows what a
bump actually rewrites.

This is not test coverage. The suite in `tests/` and the fixtures in
`src/domain/testdata/` prove correctness. These exist to be looked at.

| Path | Shows |
| --- | --- |
| [`npm/single-project`](npm/single-project) | One package, manifest and lock moving together |
| [`npm/multi-project`](npm/multi-project) | Two packages released independently, with per-project tags |
| [`cs/single-project`](cs/single-project) | `<Version>` in a `.csproj`, and what stays put around it |
| [`cs/multi-project`](cs/multi-project) | Two C# projects versioned independently |
| [`cs/shared-version`](cs/shared-version) | An executable and its library on one `Directory.Build.props` |
| [`py/single-project`](py/single-project) | One Python project, `pyproject.toml` and `uv.lock` together |
| [`rs/workspace`](rs/workspace) | Two crates sharing one `Cargo.lock`, released together |

## Git is off, deliberately

These projects have no repository of their own — they sit inside vump's. A
commit or tag made here lands in **this** repository, so every `vump.toml` in
the sandbox sets `through = "none"`.

Two further layers stand behind that setting, because it can be overridden by
a flag. vump refuses to **act on** a configuration nested inside another's
repository unless `--allow-nested` is passed — which is why every command below
carries it, `status` included: reading the wrong project is a hazard of its own,
since a check can answer about one you never meant to verify. And each
project names its own tag and commit so that one made in spite of all that
still cannot be mistaken for vump's: `sandbox-npm-v1.2.3` rather than
`v1.2.3`. The release workflow triggers on tags matching `v*`, and nothing
from here may match it.

Typing `--allow-nested` on every command here is the intended cost. These projects
are the one place the nested layout is kept deliberately, and the flag is a
standing reminder that anywhere else the answer is `[[project]]`.

That leaves nothing untested. Commits, tags and pushes are covered by the
end-to-end suite, which creates a throwaway repository per test and asserts on
what git actually built. The sandbox covers the half those tests do not show a
human: the file rewrites.

If you do want to watch the git side by hand, clone into a scratch directory
first and let the tags die with it:

```bash
git clone . /tmp/vump-scratch && cd /tmp/vump-scratch/sandbox/npm/single-project
```

Without that clone, `--allow-nested` is what lets a bump run at all, and adding
`--through tag` to it grows this repository's history by a commit and a
sandbox-prefixed tag.

## Trying it

```bash
cd sandbox/npm/single-project
vump status --allow-nested             # what is recorded now
vump minor --dry-run --allow-nested    # what a bump would rewrite
vump minor --allow-nested              # rewrite it
node index.js               # the new version, from the manifest
```

The multi-project directories address their projects by name, and a tag
identifies its own project:

```bash
cd sandbox/npm/multi-project
vump status --allow-nested                # every project at a glance
vump patch --project project-a --allow-nested
vump check sandbox-npm-project-a-v1.0.1 --allow-nested
```

For C#, the version reaches the assembly:

```bash
cd sandbox/cs/single-project
vump alpha --toward minor --allow-nested
dotnet run --project Demo            # Demo 1.1.0-alpha.0
```

One `Directory.Build.props` can carry the version for a whole solution. Neither
project below declares its own, so both inherit it and report the same number:

```bash
cd sandbox/cs/shared-version
vump minor --allow-nested            # rewrites one file, not two
dotnet run --project App             # App 1.1.0 / Lib 1.1.0
```

For Python, the lock moves with the manifest and needs no network to do it —
`uv lock` afterwards produces the same file vump already wrote:

```bash
cd sandbox/py/single-project
vump minor --allow-nested            # pyproject.toml and uv.lock
uv run python -m demo                # demo 1.1.0
```

A Cargo workspace shares one lock holding an entry per member, matched by the
name each manifest declares. Declaring both manifests is what moves them as a
unit — `--locked` is the check that vump wrote what cargo expects:

```bash
cd sandbox/rs/workspace
vump minor --allow-nested            # both manifests and the shared lock
cargo run --locked --bin sandbox-cli # sandbox-cli 1.1.0
```

## Checking every ecosystem at once

The test suite proves vump writes the bytes it meant to. Only each ecosystem's
own toolchain can say whether those bytes reach a running program:

```bash
./scripts/sandbox-smoke.sh
```

It bumps every project with the vump in this working tree, asks node, dotnet,
uv and cargo what version they build, and reverts afterwards. A toolchain that
is not installed is skipped rather than failed. It is deliberately not part of
`cargo test`, which would need all four on every CI runner to test somebody
else's tool.

## Putting it back

Experiments are meant to be thrown away:

```bash
git checkout -- sandbox
```
