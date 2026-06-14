import { defineConfig } from "vite";

// Config minima para Tauri: puerto fijo 1420, sin limpiar la consola.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "esnext",
    emptyOutDir: true,
  },
});
