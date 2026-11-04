import { defineConfig } from "vite";

// Builds the desktop consent page into the directory the API binary embeds.
// Output names are fixed (no hashes) so `include_str!` paths stay stable.
export default defineConfig({
  // Served from /desktop/login* by the API, never from the filesystem root.
  base: "/desktop/",
  build: {
    outDir: "../../crates/cloud-api/web/dist",
    emptyOutDir: true,
    rollupOptions: {
      input: "index.html",
      output: {
        entryFileNames: "login.js",
        chunkFileNames: "login.js",
        assetFileNames: (info) =>
          info.name?.endsWith(".css") ? "login.css" : "[name][extname]",
      },
    },
  },
});
