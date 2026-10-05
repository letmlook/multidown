import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    // build-extension.test.mjs 用 node:test 独立运行（npm run test:extension），
    // vitest 不参与，否则会因为找不到 vitest 用例而报错。
    exclude: [...configDefaults.exclude, "scripts/build-extension.test.mjs"],
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Rust 的 target/ 里是链接器正在写的产物（Windows 下 multidown_lib.dll 会被
      // 独占锁定）。不排除的话，Vite 的文件监听会在编译期间抛 EBUSY 并整个崩掉，
      // 前端热更新也就无从谈起。这些目录没有任何前端源码。
      ignored: [
        "**/src-tauri/target/**",
        "**/integration/*/target/**",
        "**/dist/**",
        "**/dist-extension/**",
      ],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: ["es2021", "chrome100", "safari13"],
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    outDir: "dist",
  },
});
