/**
 * Points the site at a different published vump release.
 *
 * The version the page states is a declared input, not something the build
 * discovers: it is read from src/content/release.json at build time, so every
 * deploy of the same commit says the same thing and no visitor's browser talks
 * to an API to find out. This script only makes updating it one command instead
 * of a lookup — it reads the repository's own tags, so it works offline and
 * cannot be rate-limited.
 *
 *   pnpm bump:vump          # newest stable tag
 *   pnpm bump:vump 0.6.2    # a specific one
 *
 * Pre-releases are skipped deliberately. The page's audience is someone
 * deciding whether to try vump, and the download button hands them the newest
 * stable; stating an alpha beside it would be two answers to one question.
 */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const target = fileURLToPath(new URL('../src/content/release.json', import.meta.url));

/** Parses a stable `vX.Y.Z` tag. Anything with a pre-release part is not one. */
function stable(tag) {
  const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag.trim());
  return match ? { tag, parts: [+match[1], +match[2], +match[3]] } : null;
}

function newestStable() {
  const tags = execFileSync('git', ['tag', '--list', 'v*'], { encoding: 'utf8' }).split('\n');
  const releases = tags.map(stable).filter(Boolean);

  if (releases.length === 0) {
    throw new Error('no stable v* tags in this repository; pass a version explicitly');
  }

  releases.sort((a, b) => {
    for (let i = 0; i < 3; i++) {
      if (a.parts[i] !== b.parts[i]) return a.parts[i] - b.parts[i];
    }
    return 0;
  });

  return releases.at(-1).tag.slice(1);
}

const requested = process.argv[2]?.replace(/^v/, '');
if (requested && !/^\d+\.\d+\.\d+$/.test(requested)) {
  throw new Error(`not a stable version: ${requested}`);
}

const version = requested ?? newestStable();
const current = JSON.parse(readFileSync(target, 'utf8')).version;

if (current === version) {
  process.stdout.write(`already at ${version}\n`);
} else {
  writeFileSync(target, `${JSON.stringify({ version }, null, 2)}\n`);
  process.stdout.write(`${current} -> ${version}\n`);
}
