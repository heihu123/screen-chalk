import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 需固定端口，且不使用自动打开的浏览器
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  build: {
    // 旧 dist 目录残留一个被锁定无法删除的文件，会导致 tauri 打包失败，产物改输出到 dist2
    outDir: "dist2",
    emptyOutDir: true,
  },
  server: {
    port: 1421,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});