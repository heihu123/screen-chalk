import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import NoteApp from "./NoteApp";
import "./styles.css";

// 强制 html/body 背景透明：防止 WebView2 在页面加载的任意阶段
// 渲染默认白底面板盖住整个屏幕（透明窗口的最后一层兜底）。
document.documentElement.style.background = "transparent";
document.body.style.background = "transparent";

// 按窗口 label 分流：note 窗口渲染便签界面，其余（main）渲染标注主界面。
// 非浏览器环境无 __TAURI_INTERNALS__，降级渲染主界面。
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const windowLabel = isTauri ? getCurrentWebviewWindow().label : "";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {windowLabel === "note" ? <NoteApp /> : <App />}
  </React.StrictMode>
);
