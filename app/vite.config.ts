import react from '@vitejs/plugin-react';
import type { Plugin } from 'vite';
import { defineConfig } from 'vite';

// Vite adds `crossorigin` to the entry <script type="module"> and
// preload <link> tags by default (a CDN-safety default). Under Tauri's
// custom `tauri://` asset protocol that turns the module load into a
// CORS request, which — unlike a normal http(s) origin — can fail
// silently in the webview with no visible error, leaving the window
// blank. Stripping it is the standard fix for a Tauri + Vite app.
function stripCrossorigin(): Plugin {
  return {
    name: 'strip-crossorigin-for-tauri-protocol',
    transformIndexHtml(html) {
      return html.replace(/\s+crossorigin(="[^"]*")?/g, '');
    },
  };
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), stripCrossorigin()],
  // Matches tauri.conf.json's build.devUrl so `tauri dev` finds it.
  server: {
    port: 1420,
    strictPort: true,
  },
});
