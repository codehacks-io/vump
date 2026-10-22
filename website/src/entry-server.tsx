import { renderToString } from 'react-dom/server';

import { App } from './App.tsx';

/** Rendered at build time by scripts/prerender.mjs. */
export function render(): string {
  return renderToString(<App />);
}
