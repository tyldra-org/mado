/// <reference types="vitest/config" />
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vite";

// The build goes into the app's bundled resources (mado-docs/spec/15 section 2).
export default defineConfig({
  // svelteTesting only acts under Vitest: it resolves Svelte's browser build
  // and cleans the DOM after each test.
  plugins: [svelte(), svelteTesting()],
  build: {
    outDir: "../app/resources/ui",
    emptyOutDir: true,
    target: "es2022",
    // One page per webview kind (spec 01 §5.1, 06 §9, 01 §4.4).
    rollupOptions: {
      input: {
        window: "window.html",
        overlay: "overlay.html",
        settings: "settings.html",
        picker: "picker.html",
        internal: "internal.html",
        // Opened only by development builds (M1-T10).
        "dev-platform": "dev-platform.html",
      },
    },
  },
  clearScreen: false,
  test: {
    // Component and runes tests are *.svelte.test.ts and run here. Plain
    // *.test.ts files belong to bun test (see bunfig.toml at the root).
    include: ["src/**/*.svelte.test.ts"],
    environment: "jsdom",
  },
});
