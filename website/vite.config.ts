import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

/**
 * Served from vump.codehacks.io, at the root, so there is no base path to
 * configure. A bare github.io project page would need one, at /<repo>/ — that
 * is not this site's URL, and the custom domain is set in the repository's
 * Pages settings rather than in a CNAME file, since the deploy is a workflow.
 */
export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    // Every visitor loads the whole page, so splitting it into chunks buys
    // nothing and costs a round trip.
    assetsInlineLimit: 2048,
  },
});
