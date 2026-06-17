import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 19773, strictPort: true },
  build: {
    rollupOptions: {
      input: {
        avatar: resolve(__dirname, "avatar.html"),
        dashboard: resolve(__dirname, "dashboard.html"),
      },
    },
  },
});
