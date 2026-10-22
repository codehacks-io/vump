# vump website

The landing page for [vump](https://github.com/codehacks-io/vump). One page, built
to static files and destined for GitHub Pages.

```bash
pnpm install
pnpm dev             # http://localhost:5173
pnpm build && pnpm preview   # the real static output, prerender included
pnpm check           # typecheck, lint, format, build — what CI runs
```

`pnpm dev` never runs the prerender step: `index.html` ships an empty `#root`
there and the app client-renders into it. To see what a crawler or a scripting-
disabled browser actually receives, build and preview.

| Script                    | Does                                                          |
| ------------------------- | ------------------------------------------------------------- |
| `dev`                     | Vite dev server with hot reload                               |
| `build`                   | Type-checks, then writes `dist/`                              |
| `preview`                 | Serves `dist/` as a deploy would                              |
| `typecheck`               | `tsc -b`, no emit                                             |
| `lint`                    | oxlint; any warning fails                                     |
| `format` / `format:check` | Prettier over everything                                      |
| `check`                   | All four, in order                                            |
| `og`                      | Regenerates `public/og.png` and `public/apple-touch-icon.png` |

## What is where

| Path                       | Holds                                                                                                                   |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `src/content/`             | Every fact the page states: terminal runs, tracked files, snippets, links. Nothing here renders.                        |
| `src/components/sections/` | One file per section, in page order.                                                                                    |
| `src/components/ui/`       | The primitives: terminal pane, code block, section frame, reveal, buttons.                                              |
| `src/lib/`                 | Hooks and the two small engines — a segment model for command output, and a highlighter for the four snippet languages. |
| `src/index.css`            | The whole design system: palette, type, motion.                                                                         |
| `scripts/og.mjs`           | Renders the social card from the same values, offline.                                                                  |

## What this page is for

It introduces vump to someone who has never seen it: what it does, whether it
handles their files, and how to start. That is all.

**It does not restate the documentation.** No flag tables, no exit codes, no
configuration reference, no design rationale — those live in the repository and
are linked from here. Anything copied onto this page is a second copy to keep
true, and the page is not worth that.

**Content is separate from layout on purpose.** Every terminal pane reproduces
what the binary actually prints — the marks, the column alignment, the wording
of the errors. When vump's output changes, `src/content/runs.ts` is the only
file to revisit.

**The page states no version number.** The release chip fetches the newest
release from the GitHub API as the page loads, and renders nothing when that
fails. A version written into a marketing site is a version nobody bumps, which
is the defect vump exists to prevent.

## Design system

Colours are semantic roles — `ground`, `ink`, `muted`, `line`, `signal` — defined
once in `src/index.css` and flipped by a `data-theme` attribute on the document
element. No component names a literal colour, so the two themes cannot drift
apart. A script in `index.html` resolves the theme before first paint; an
explicit choice outranks the system preference.

Terminal surfaces are held dark in both themes. Rendered command output should
read as output, not as a styled quotation.

The accent is spent on one thing: what changed. A new version number, a passing
mark, a highlighted line in a snippet. Red marks a failure, and nothing else is
coloured at all.

## Lint configuration

Four rules are off in `.oxlintrc.json`, each because it does not fit this
codebase rather than because it was inconvenient:

| Rule                       | Why                                                                                                                        |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `react/no-array-index-key` | Rendered lines and tokens are static arrays that never reorder; the index is the correct identity.                         |
| `max-lines-per-function`   | Counts JSX as logic. A section component is markup, not complexity.                                                        |
| `import/max-dependencies`  | `App.tsx` composes every section; that is its whole job.                                                                   |
| `require-unicode-regexp`   | The highlighter's patterns run over ASCII snippets written in this repository, and `u` mode outlaws the escapes they need. |

Everything else — correctness, suspicious, perf and pedantic — is on, and a
warning fails the run.

## The version the page states

The page says which vump release it describes — in the masthead, and on the
download button. That value is read at build time from
[`src/content/release.json`](src/content/release.json), a declared input like
any other.

It is not fetched from the GitHub API in the visitor's browser. That call is
rate-limited to sixty an hour per address, so the version would simply vanish
for anyone behind a busy NAT, and the page would say different things at
different moments to different people.

It is not read from `Cargo.toml` either, tempting as that is. `Cargo.toml`
holds the version most recently bumped, which is not the same as the version
most recently _published_ — a bump that has been tagged locally but not pushed
leaves it describing a release that does not exist, with no binaries behind the
download button.

Pointing the site at a newer release is deliberate and offline — the script
reads the repository's own tags and skips pre-releases, since the button beside
the number hands over the newest stable:

```bash
pnpm bump:vump          # newest stable tag
pnpm bump:vump 0.6.2    # a specific one
```

Commit the change like any other, then cut a website release to deploy it.

## Two release lines

The binary and the website ship independently, as two vump projects declared in
the repository's [`vump.toml`](../vump.toml):

| Project   | Tracks                     | Tagged           | Workflow                                                |
| --------- | -------------------------- | ---------------- | ------------------------------------------------------- |
| `main`    | `Cargo.toml`, `Cargo.lock` | `v1.2.3`         | `release.yml` — builds, signs and publishes the binary  |
| `website` | `website/package.json`     | `website-v1.2.3` | `release-website.yml` — deploys this directory to Pages |

Each tag shape triggers exactly one of them, and `vump check` infers which
project a pushed tag describes from its shape, so neither workflow has to name
`--project`. Shipping a site-only change is:

```bash
vump patch --project website --through push
```

No vump version, no release notes, nothing said to anyone pinning the binary.

The reverse holds too: releasing the binary does not redeploy the site. So the
version the page states can lag a release until the site is deployed again,
which is the deliberate cost of not having one tag do two unrelated things.

## Deployment

`release-website.yml` builds this directory and publishes `dist/` to GitHub
Pages, gated on `vump check` agreeing that the tag matches
`website/package.json`. The build receives `VITE_WEBSITE_BUILD_SHA`, which is
what the badge in the corner of the page reports alongside the site's version —
a bug report is a screenshot, and that is the pair needed to know which build
it came from.

The install step is `pnpm install --frozen-lockfile`: the committed
`pnpm-lock.yaml` is the deploy's declared input, so nothing is resolved or
discovered at deploy time and a rebuild of the same commit produces the same
bytes.

Served from **vump.codehacks.io**, configured in the repository's Pages
settings rather than by a `CNAME` file — a workflow-based deploy does not need
one. The site is therefore always at the root, which is why there is no Vite
`base` path here; a bare `github.io/<repo>/` project page would need one.

## Prerendering

`dist/index.html` ships with the page already rendered into it. `vite build`
produces the client bundle, a second pass builds `src/entry-server.tsx` to
`dist-ssr/`, and `scripts/prerender.mjs` inlines that HTML into `#root` and
deletes the intermediate. `src/main.tsx` hydrates when `#root` already has
children and renders normally when it does not, which is what `pnpm dev`
serves.

The entrance animations are gated on a `js` class the pre-paint script adds, so
a client that never runs the script sees the prerendered page as it stands
rather than a blank one waiting for a reveal that will never come.
