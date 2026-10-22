import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

/**
 * Separate from vite.config.ts because this build's only output is the HTML
 * string entry-server.tsx returns. It needs the JSX transform and nothing else:
 * Tailwind's plugin compiles a stylesheet the client build has already emitted,
 * and pointing an --ssr entry at a config carrying it produces a second copy
 * nothing loads. See scripts/prerender.mjs.
 */
export default defineConfig({
  plugins: [react()],
});
