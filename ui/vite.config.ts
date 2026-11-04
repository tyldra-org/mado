import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// The build goes into the app's bundled resources (mado-docs/spec/15 section 2).
export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "../app/resources/ui",
    emptyOutDir: true,
    target: "es2022",
  },
  clearScreen: false,
});
