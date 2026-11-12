import { resolve } from "node:path";
import { defineConfig } from "vite";

// Two pages: her overlay (one per monitor) and the settings window.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "es2022",
    rollupOptions: {
      input: {
        overlay: resolve(import.meta.dirname, "overlay.html"),
        settings: resolve(import.meta.dirname, "settings.html"),
      },
    },
  },
});
