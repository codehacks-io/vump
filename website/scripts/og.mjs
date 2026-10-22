/**
 * Renders the social card and the touch icon from the same values the site
 * uses, with no browser and no network.
 *
 * The card is generated rather than drawn by hand so it cannot drift from the
 * page it advertises: change the headline here and both move together. Fonts
 * come out of node_modules, decompressed from woff2 because the renderer reads
 * TrueType only.
 */

import { Resvg } from '@resvg/resvg-js';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import wawoff2 from 'wawoff2';

const root = fileURLToPath(new URL('..', import.meta.url));

const FONTS = [
  '@fontsource-variable/instrument-sans/files/instrument-sans-latin-wght-normal.woff2',
  '@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2',
  '@fontsource/instrument-serif/files/instrument-serif-latin-400-italic.woff2',
];

const INK = '#f2f4f6';
const MUTED = '#9aa3ae';
const FAINT = '#6c7481';
const SIGNAL = '#5ee0bd';
const GROUND = '#121419';
const PANE = '#171a20';

// The check is drawn rather than typed: U+2713 is outside the latin subset the
// site loads, and a font that does not carry it renders a notdef box.
const RUN = [
  { text: '$ vump patch --through tag', fill: INK },
  { text: '0.2.0 -> 0.2.1', fill: SIGNAL, ok: true },
  { text: '  Cargo.toml', fill: FAINT, indent: true },
  { text: '  Cargo.lock', fill: FAINT, indent: true },
  { text: 'tagged     v0.2.1', fill: SIGNAL, ok: true },
];

function check(x, y) {
  return `<path d="M${x} ${y} l4 4 7.5-8" fill="none" stroke="${SIGNAL}" stroke-width="2.1"
                stroke-linecap="round" stroke-linejoin="round" />`;
}

function grid(width, height, step) {
  const lines = [];
  for (let x = step; x < width; x += step) {
    lines.push(`<line x1="${x}" y1="0" x2="${x}" y2="${height}" />`);
  }
  for (let y = step; y < height; y += step) {
    lines.push(`<line x1="0" y1="${y}" x2="${width}" y2="${y}" />`);
  }
  return `<g stroke="#ffffff" stroke-opacity="0.05" stroke-width="1">${lines.join('')}</g>`;
}

function mark(x, y, size) {
  const s = size / 32;
  return `<g transform="translate(${x} ${y}) scale(${s})">
    <rect width="32" height="32" rx="7" fill="#ffffff" fill-opacity="0.07" />
    <path d="M9.5 17.5 16 11l6.5 6.5" fill="none" stroke="${SIGNAL}" stroke-width="2.6"
          stroke-linecap="round" stroke-linejoin="round" />
    <path d="M9.5 22.5h13" fill="none" stroke="#ffffff" stroke-opacity="0.35" stroke-width="2.2"
          stroke-linecap="round" />
  </g>`;
}

const card = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  <defs>
    <radialGradient id="glow" cx="50%" cy="0%" r="70%">
      <stop offset="0%" stop-color="${SIGNAL}" stop-opacity="0.16" />
      <stop offset="100%" stop-color="${SIGNAL}" stop-opacity="0" />
    </radialGradient>
  </defs>

  <rect width="1200" height="630" fill="${GROUND}" />
  ${grid(1200, 630, 60)}
  <rect width="1200" height="630" fill="url(#glow)" />

  ${mark(72, 64, 40)}
  <text x="126" y="95" font-family="Instrument Sans" font-size="34" fill="${INK}"
        letter-spacing="-0.5">vump</text>

  <text x="72" y="272" font-family="Instrument Sans" font-size="64" fill="${INK}"
        letter-spacing="-2.4">One version.</text>
  <text x="72" y="344" font-family="Instrument Sans" font-size="64" fill="${INK}"
        letter-spacing="-2.4">Every file.</text>
  <text x="72" y="416" font-family="Instrument Serif" font-style="italic" font-size="64"
        fill="${SIGNAL}" letter-spacing="-1">One command.</text>

  <text x="72" y="486" font-family="Instrument Sans" font-size="23" fill="${MUTED}">
    Bump every file that records your version at once — then
  </text>
  <text x="72" y="518" font-family="Instrument Sans" font-size="23" fill="${MUTED}">
    commit, tag, and verify the tag in CI.
  </text>

  <line x1="72" y1="566" x2="1128" y2="566" stroke="#ffffff" stroke-opacity="0.1" />
  <text x="72" y="596" font-family="JetBrains Mono" font-size="17" fill="${FAINT}"
        letter-spacing="1.4">VUMP.CODEHACKS.IO</text>

  <g transform="translate(716 168)">
    <rect width="412" height="300" rx="14" fill="${PANE}" stroke="#ffffff" stroke-opacity="0.1" />
    <line x1="0" y1="46" x2="412" y2="46" stroke="#ffffff" stroke-opacity="0.08" />
    <text x="24" y="30" font-family="JetBrains Mono" font-size="14" fill="${FAINT}"
          letter-spacing="1.2">~/widget</text>
    ${RUN.map((line, index) => {
      const y = 94 + index * 34;
      const x = line.ok || line.indent ? 48 : 24;
      const glyph = line.ok ? check(25, y - 7) : '';
      return `${glyph}<text x="${x}" y="${y}" font-family="JetBrains Mono" font-size="17"
               fill="${line.fill}" xml:space="preserve">${line.text}</text>`;
    }).join('\n    ')}
  </g>
</svg>`;

async function loadFonts() {
  const dir = await mkdtemp(join(tmpdir(), 'vump-og-'));
  const paths = [];

  // Strictly one at a time. The decompressor is a single emscripten module with
  // its own heap, and overlapping calls hand back corrupted TrueType: the card
  // then renders every line in whichever face survived, differently on each
  // run. Awaiting in the loop is the fix, not the smell.
  for (const font of FONTS) {
    // oxlint-disable-next-line no-await-in-loop
    const woff2 = await readFile(join(root, 'node_modules', font));
    // oxlint-disable-next-line no-await-in-loop
    const ttf = Buffer.from(await wawoff2.decompress(woff2));
    const path = join(dir, `${font.split('/').pop()?.replace('.woff2', '')}.ttf`);
    // oxlint-disable-next-line no-await-in-loop
    await writeFile(path, ttf);
    paths.push(path);
  }

  return { dir, paths };
}

function render(svg, width, fontFiles) {
  return new Resvg(svg, {
    fitTo: { mode: 'width', value: width },
    font: { fontFiles, loadSystemFonts: false, defaultFontFamily: 'Instrument Sans' },
  })
    .render()
    .asPng();
}

const { dir, paths } = await loadFonts();

try {
  await mkdir(join(root, 'public'), { recursive: true });

  await writeFile(join(root, 'public/og.png'), render(card, 1200, paths));

  const favicon = await readFile(join(root, 'public/favicon.svg'), 'utf8');
  await writeFile(join(root, 'public/apple-touch-icon.png'), render(favicon, 180, paths));

  process.stdout.write('wrote public/og.png and public/apple-touch-icon.png\n');
} finally {
  await rm(dir, { recursive: true, force: true });
}
