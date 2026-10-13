# ScreenChalk

基于 React、TypeScript、Vite 和 Tauri 2 的桌面屏幕标注工具。

## 项目结构

- `screen-chalk/`：基础版本。
- `screen-chalk-magnify/`：增强版本，包含放大镜、便签和鼠标效果相关模块。

两个目录都是独立项目，均保留原始源码、依赖锁文件与应用图标。

## 本地运行

安装 Node.js、Rust 和对应系统的 Tauri 2 开发依赖，然后在需要运行的版本目录中执行：

```sh
npm ci
npm run tauri dev
```

## 构建

```sh
npm run build
npm run tauri build
```

增强版前端构建目录为 `dist2`，与其 Tauri 配置保持一致。

仓库不包含依赖目录、编译缓存及本地环境文件。
