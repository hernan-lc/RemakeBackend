import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { demoPlugin } from "../admin/demo/plugin";

// Builds the desktop consent page into the directory the API binary embeds.
// Output names are fixed (no hashes) so `include_str!` paths stay stable.
export default defineConfig(({ mode, command }) => {
  if (mode === "demo" && command !== "serve") throw new Error("Demo is development-only.");
  return {
    plugins: [vue(), ...(mode === "demo" ? [demoPlugin()] : [])],
    server: mode === "demo" ? { host: "127.0.0.1", port: 5174, strictPort: true } : undefined,
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
  };
});
