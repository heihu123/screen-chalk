import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 需固定端口，且不使用自动打开的浏览器
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});