/**
 * Inlines the app's rendered HTML into dist/index.html.
 *
 * Runs after the client and SSR builds. Without it the deployed page ships an
 * empty #root: correct in a browser, nothing at all to a crawler, a link
 * preview, or an agent fetching the page — and this site's whole job is being
 * read by someone deciding whether to try the tool.
 *
 * dist-ssr/ is a build-time intermediate and is deleted here rather than
 * deployed.
 */

import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteDir = dirname(dirname(fileURLToPath(import.meta.url)));
const distDir = join(websiteDir, 'dist');
const ssrDir = join(websiteDir, 'dist-ssr');

const { render } = await import(join(ssrDir, 'entry-server.js'));

const indexPath = join(distDir, 'index.html');
const template = readFileSync(indexPath, 'utf8');
const rendered = template.replace('<div id="root"></div>', `<div id="root">${render()}</div>`);

if (rendered === template) {
  throw new Error(`prerender: no '<div id="root"></div>' placeholder in ${indexPath}`);
}

writeFileSync(indexPath, rendered);
rmSync(ssrDir, { recursive: true, force: true });

process.stdout.write('prerendered dist/index.html\n');
