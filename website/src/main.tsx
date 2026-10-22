// Variable faces ship one stylesheet per axis, covering every subset; the
// browser fetches only the subset it needs, gated by unicode-range. The static
// serif is imported latin-only, which is all this site is written in.
import '@fontsource-variable/instrument-sans/wght.css';
import '@fontsource-variable/jetbrains-mono/wght.css';
import '@fontsource/instrument-serif/latin-400.css';
import '@fontsource/instrument-serif/latin-400-italic.css';
import './index.css';

import { StrictMode } from 'react';
import { createRoot, hydrateRoot } from 'react-dom/client';

import { App } from './App.tsx';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root');

const app = (
  <StrictMode>
    <App />
  </StrictMode>
);

// The build's prerender step fills #root with real markup, so a production load
// hydrates it instead of throwing it away. `vite dev` serves index.html
// unprerendered, where #root is empty and a plain render is what is wanted.
if (root.hasChildNodes()) {
  hydrateRoot(root, app);
} else {
  createRoot(root).render(app);
}
