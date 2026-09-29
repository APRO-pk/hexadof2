import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config for the HexaDOF Tauri frontend.
// Tauri expects a fixed port and does not want vite to obscure rust errors.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5183,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome110",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 2000,
  },
});
