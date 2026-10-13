import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/** 单条待办 */
interface TodoItem {
  id: string;
  text: string;
  done: boolean;
}

/** 一页便签：文本模式（备忘/整理思路）或清单模式（todo） */
interface NotePage {
  id: string;
  name: string;
  mode: "text" | "todo";
  /** 文本模式内容（HTML；旧版本数据为纯文本，加载时自动转换） */
  text: string;
  /** text 字段是否为富文本 HTML（旧数据无此字段 → 视为纯文本） */
  rich: boolean;
  todos: TodoItem[];
}

/** 窗口状态（物理像素，与 Rust 端恢复窗口时一致） */
interface NoteWindowState {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** notes.json 的完整结构 */
interface NotesData {
  pages: NotePage[];
  activePageId: string;
  window?: NoteWindowState;
}

const MAX_PAGES = 8;
const MAX_NAME_LEN = 12;
/** 内容变更后的写盘防抖间隔 */
const SAVE_DEBOUNCE_MS = 400;
/** 窗口移动/缩放后的状态记录防抖间隔 */
const WIN_STATE_DEBOUNCE_MS = 600;

function uid(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `id-${Date.now()}-${Math.round(Math.random() * 1e6)}`;
}

function newPage(name: string): NotePage {
  return { id: uid(), name, mode: "text", text: "", rich: true, todos: [] };
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** 旧版本便签存的是纯文本：逐行转成 div 块，保住原有换行结构 */
function plainTextToHtml(text: string): string {
  if (!text) return "";
  return text
    .split("\n")
    .map((line) => (line ? `<div>${escapeHtml(line)}</div>` : "<div><br></div>"))
    .join("");
}

/** 解析后端返回的 JSON；结构异常时回退一页空白便签，绝不让坏数据把界面卡死 */
function parseNotes(raw: string): NotesData {
  try {
    const v = JSON.parse(raw) as Partial<NotesData>;
    if (Array.isArray(v.pages) && v.pages.length > 0) {
      const pages = v.pages.map((p) => {
        const rawText = typeof p?.text === "string" ? p.text : "";
        return {
          id: p?.id ?? uid(),
          name: p?.name ?? "便签",
          mode: p?.mode === "todo" ? ("todo" as const) : ("text" as const),
          // rich=true 说明已是 HTML 原样使用；旧的纯文本数据按行转成 HTML 块
          text: p?.rich === true ? rawText : plainTextToHtml(rawText),
          rich: true,
          todos: Array.isArray(p?.todos)
            ? p.todos.map((t) => ({
                id: t?.id ?? uid(),
                text: typeof t?.text === "string" ? t.text : "",
                done: !!t?.done,
              }))
            : [],
        };
      });
      const activePageId = pages.some((p) => p.id === v.activePageId)
        ? (v.activePageId as string)
        : pages[0].id;
      const data: NotesData = { pages, activePageId };
      const w = v.window;
      if (
        w &&
        [w.x, w.y, w.w, w.h].every((n) => typeof n === "number" && Number.isFinite(n))
      ) {
        data.window = w;
      }
      return data;
    }
  } catch (e) {
    console.error("parseNotes failed", e);
  }
  const page = newPage("便签 1");
  return { pages: [page], activePageId: page.id };
}

/** 窗口里最小的勾选图标 */
function CheckIcon() {
  return (
    <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true">
      <path
        d="M2 6.5 4.8 9 10 3"
        fill="none"
        stroke="#fff"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** 删除本页的垃圾桶图标 */
function TrashIcon() {
  return (
    <svg viewBox="0 0 14 14" width="12" height="12" aria-hidden="true">
      <path
        d="M2.5 4h9M5.5 4V2.8h3V4M4 4l.5 8h5l.5-8"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * 文本模式的轻量富文本编辑器：标题/正文/序号列表/点列。
 * 编辑器为非受控组件（内容直接在 DOM 里改），仅挂载时装载一次 HTML；
 * 切页重建由外层 key={page.id} 保证。格式按钮 mousedown 阻止默认行为，
 * 点击时不抢编辑器焦点，命令作用于当前光标所在段落。
 */
function RichTextEditor({
  html,
  onChange,
}: {
  html: string;
  onChange: (html: string) => void;
}) {
  const editorRef = useRef<HTMLDivElement>(null);
  /** 当前光标所在块格式："h2"=标题，其余一律视为正文 */
  const [block, setBlock] = useState<"h2" | "p">("p");
  const [inOL, setInOL] = useState(false);
  const [inUL, setInUL] = useState(false);

  // 仅挂载时装载初始内容；之后的输入直接读 DOM，不走 React 重渲染
  useEffect(() => {
    const el = editorRef.current;
    if (el) {
      el.innerHTML = html;
      el.classList.toggle("is-empty", el.textContent === "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 光标移动/选区变化时同步格式按钮的高亮状态
  useEffect(() => {
    const sync = () => {
      const el = editorRef.current;
      if (!el || !el.contains(document.activeElement)) return;
      const b = String(document.queryCommandValue("formatBlock")).toLowerCase();
      setBlock(b === "h2" ? "h2" : "p");
      setInOL(document.queryCommandState("insertOrderedList"));
      setInUL(document.queryCommandState("insertUnorderedList"));
    };
    document.addEventListener("selectionchange", sync);
    return () => document.removeEventListener("selectionchange", sync);
  }, []);

  // 执行编辑命令（WebView2/Chromium 对 execCommand 支持完整）。
  // 命令会触发 input 事件，内容保存统一走 onInput，这里不重复上报
  const exec = (cmd: string, value?: string) => {
    const el = editorRef.current;
    if (!el) return;
    el.focus();
    document.execCommand(cmd, false, value);
  };

  /** 标题：再点一次切回正文，形成开关手感 */
  const toggleHeading = () => {
    const next = block === "h2" ? "p" : "h2";
    exec("formatBlock", next);
    setBlock(next);
  };

  /** 正文：先退出列表（若在列表中），再落回普通段落 */
  const applyBody = () => {
    if (inOL) exec("insertOrderedList");
    if (inUL) exec("insertUnorderedList");
    if (block !== "p") exec("formatBlock", "p");
    setBlock("p");
  };

  return (
    <>
      <div className="note-fmt">
        <button
          className={block === "h2" ? "on" : ""}
          title="标题（再点一次回到正文）"
          onMouseDown={(e) => e.preventDefault()}
          onClick={toggleHeading}
        >
          标题
        </button>
        <button
          title="正文（清除标题/列表格式）"
          onMouseDown={(e) => e.preventDefault()}
          onClick={applyBody}
        >
          正文
        </button>
        <span className="note-fmt-sep" />
        <button
          className={`note-fmt-num${inOL ? " on" : ""}`}
          title="序号列表"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => exec("insertOrderedList")}
        >
          1.
        </button>
        <button
          className={`note-fmt-dot${inUL ? " on" : ""}`}
          title="点列"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => exec("insertUnorderedList")}
        >
          •
        </button>
      </div>
      <div
        ref={editorRef}
        className="note-editor"
        contentEditable
        suppressContentEditableWarning
        spellCheck={false}
        data-placeholder="写点什么…备忘、思路、临时记录"
        onInput={(e) => {
          const el = e.currentTarget;
          el.classList.toggle("is-empty", el.textContent === "");
          onChange(el.innerHTML);
        }}
        onKeyDown={(e) => {
          // 标题里回车：新起一行自动回到正文（Shift+Enter 仍是标题内换行）
          if (e.key === "Enter" && !e.shiftKey && block === "h2") {
            requestAnimationFrame(() => {
              exec("formatBlock", "p");
              setBlock("p");
            });
          }
        }}
        onPaste={(e) => {
          // 只贴纯文本，避免从网页复制的样式污染便签
          e.preventDefault();
          const text = e.clipboardData.getData("text/plain");
          if (text) exec("insertText", text);
        }}
      />
    </>
  );
}

/** 便签窗口根组件：只在 label=note 的窗口中渲染（见 main.tsx 分流） */
export default function NoteApp() {
  const [data, setData] = useState<NotesData | null>(null);
  /** 正在重命名的页 id */
  const [renamingId, setRenamingId] = useState<string | null>(null);
  /** 重命名草稿（Esc 取消时丢弃，不直接改页名） */
  const [renameDraft, setRenameDraft] = useState("");
  /** 清单模式底部输入框内容 */
  const [draftTodo, setDraftTodo] = useState("");

  const dataRef = useRef<NotesData | null>(null);
  const loadedRef = useRef(false);
  const saveTimer = useRef<number | null>(null);
  const renamingRef = useRef(false);
  const addInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    renamingRef.current = renamingId !== null;
  }, [renamingId]);

  const activePage =
    data?.pages.find((p) => p.id === data.activePageId) ?? data?.pages[0];

  // ---- 保存：内容变更后 debounce 写盘；窗口隐藏/关闭时立即落盘 ----

  const flushSave = useCallback(() => {
    if (saveTimer.current !== null) {
      window.clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
    const d = dataRef.current;
    if (!d) return;
    invoke("save_notes", { data: JSON.stringify(d) }).catch((e) =>
      console.error("save_notes failed", e)
    );
  }, []);

  const scheduleSave = useCallback(() => {
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(flushSave, SAVE_DEBOUNCE_MS);
  }, [flushSave]);

  // 任何状态变更（内容/页签/窗口状态）统一走这里触发防抖保存
  useEffect(() => {
    dataRef.current = data;
    if (loadedRef.current && data) scheduleSave();
  }, [data, scheduleSave]);

  // ---- 挂载：加载内容 + 窗口位置/尺寸记忆 + Esc 隐藏 + 隐藏时落盘 ----
  useEffect(() => {
    invoke<string>("get_notes")
      .then((raw) => {
        const parsed = parseNotes(raw);
        loadedRef.current = true;
        dataRef.current = parsed;
        setData(parsed);
      })
      .catch((e) => {
        console.error("get_notes failed", e);
        const page = newPage("便签 1");
        const parsed: NotesData = { pages: [page], activePageId: page.id };
        loadedRef.current = true;
        dataRef.current = parsed;
        setData(parsed);
      });

    const win = getCurrentWebviewWindow();

    // 窗口移动/缩放后记录位置（物理像素），写入数据并落盘
    let stateTimer: number | null = null;
    const queueWinState = () => {
      if (stateTimer !== null) window.clearTimeout(stateTimer);
      stateTimer = window.setTimeout(async () => {
        try {
          const pos = await win.outerPosition();
          const size = await win.outerSize();
          const d = dataRef.current;
          if (!d) return;
          const next: NotesData = {
            ...d,
            window: { x: pos.x, y: pos.y, w: size.width, h: size.height },
          };
          dataRef.current = next;
          setData(next);
        } catch (e) {
          console.error("read window state failed", e);
        }
      }, WIN_STATE_DEBOUNCE_MS);
    };
    const unMovedP = win.onMoved(queueWinState);
    const unResizedP = win.onResized(queueWinState);

    // 页面隐藏（热键/关闭按钮）或即将卸载时立即落盘，
    // 防止防抖未到期内容丢失
    const onVisibility = () => {
      if (document.visibilityState === "hidden") flushSave();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("beforeunload", flushSave);

    // Esc：正在重命名则取消重命名，否则隐藏窗口
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      if (renamingRef.current) {
        setRenamingId(null);
        return;
      }
      win.hide().catch((err) => console.error("hide failed", err));
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      if (stateTimer !== null) window.clearTimeout(stateTimer);
      unMovedP.then((u) => u());
      unResizedP.then((u) => u());
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("beforeunload", flushSave);
      window.removeEventListener("keydown", onKeyDown);
    };
    // flushSave 是稳定引用，仅挂载时执行一次
  }, [flushSave]);

  // ---- 页操作 ----

  const updatePage = useCallback((id: string, patch: Partial<NotePage>) => {
    setData((d) =>
      d
        ? { ...d, pages: d.pages.map((p) => (p.id === id ? { ...p, ...patch } : p)) }
        : d
    );
  }, []);

  const switchPage = useCallback((id: string) => {
    setData((d) => (d ? { ...d, activePageId: id } : d));
  }, []);

  const addPage = useCallback(() => {
    setData((d) => {
      if (!d || d.pages.length >= MAX_PAGES) return d;
      const page = newPage(`便签 ${d.pages.length + 1}`);
      return { ...d, pages: [...d.pages, page], activePageId: page.id };
    });
  }, []);

  const deletePage = useCallback((id: string) => {
    setData((d) => {
      if (!d || d.pages.length <= 1) return d;
      const idx = d.pages.findIndex((p) => p.id === id);
      if (idx < 0) return d;
      const pages = d.pages.filter((p) => p.id !== id);
      const activePageId =
        d.activePageId === id ? pages[Math.max(0, idx - 1)].id : d.activePageId;
      return { ...d, pages, activePageId };
    });
  }, []);

  const beginRename = useCallback((page: NotePage) => {
    setRenameDraft(page.name);
    setRenamingId(page.id);
  }, []);

  const commitRename = useCallback(
    (id: string) => {
      const name = renameDraft.trim().slice(0, MAX_NAME_LEN);
      if (name) updatePage(id, { name });
      setRenamingId(null);
    },
    [renameDraft, updatePage]
  );

  // ---- 清单操作 ----

  const addTodo = useCallback(() => {
    const text = draftTodo.trim();
    if (!text || !activePage) return;
    updatePage(activePage.id, {
      todos: [...activePage.todos, { id: uid(), text, done: false }],
    });
    setDraftTodo("");
  }, [activePage, draftTodo, updatePage]);

  const toggleTodo = useCallback(
    (pageId: string, itemId: string) => {
      const page = data?.pages.find((p) => p.id === pageId);
      if (!page) return;
      updatePage(pageId, {
        todos: page.todos.map((t) => (t.id === itemId ? { ...t, done: !t.done } : t)),
      });
    },
    [data, updatePage]
  );

  const updateTodoText = useCallback(
    (pageId: string, itemId: string, text: string) => {
      const page = data?.pages.find((p) => p.id === pageId);
      if (!page) return;
      updatePage(pageId, {
        todos: page.todos.map((t) => (t.id === itemId ? { ...t, text } : t)),
      });
    },
    [data, updatePage]
  );

  const removeTodo = useCallback(
    (pageId: string, itemId: string) => {
      const page = data?.pages.find((p) => p.id === pageId);
      if (!page) return;
      updatePage(pageId, { todos: page.todos.filter((t) => t.id !== itemId) });
    },
    [data, updatePage]
  );

  const hideWindow = useCallback(() => {
    getCurrentWebviewWindow()
      .hide()
      .catch((e) => console.error("hide failed", e));
  }, []);

  // ---- 渲染 ----

  // 内容加载完成前保持全透明，避免白底闪烁
  if (!data || !activePage) return null;

  return (
    <div className="note-root">
      <header className="note-header" data-tauri-drag-region>
        <div className="note-tabs">
          {data.pages.map((p) =>
            renamingId === p.id ? (
              <input
                key={p.id}
                className="note-tab-input"
                autoFocus
                maxLength={MAX_NAME_LEN}
                value={renameDraft}
                onChange={(e) => setRenameDraft(e.target.value)}
                onBlur={() => commitRename(p.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitRename(p.id);
                  } else if (e.key === "Escape") {
                    // 窗口级 Esc 监听会跳过重命名态，这里自行取消
                    e.stopPropagation();
                    setRenamingId(null);
                  }
                }}
              />
            ) : (
              <button
                key={p.id}
                className={`note-tab${p.id === activePage.id ? " active" : ""}`}
                onClick={() => switchPage(p.id)}
                onDoubleClick={() => beginRename(p)}
                title="双击重命名"
              >
                {p.name}
              </button>
            )
          )}
          {data.pages.length < MAX_PAGES && (
            <button className="note-tab-add" title="新建一页" onClick={addPage}>
              +
            </button>
          )}
        </div>
        <button
          className="note-close"
          title="隐藏（Esc）"
          onClick={hideWindow}
        >
          ×
        </button>
      </header>

      <div className="note-toolbar">
        <div className="note-mode">
          <button
            className={activePage.mode === "text" ? "on" : ""}
            onClick={() => updatePage(activePage.id, { mode: "text" })}
          >
            文本
          </button>
          <button
            className={activePage.mode === "todo" ? "on" : ""}
            onClick={() => updatePage(activePage.id, { mode: "todo" })}
          >
            清单
          </button>
        </div>
        {data.pages.length > 1 && (
          <button
            className="note-page-del"
            title="删除本页"
            onClick={() => deletePage(activePage.id)}
          >
            <TrashIcon />
          </button>
        )}
      </div>

      {activePage.mode === "text" ? (
        <RichTextEditor
          key={activePage.id}
          html={activePage.text}
          onChange={(html) => updatePage(activePage.id, { text: html })}
        />
      ) : (
        <>
          <div className="note-todos">
            {activePage.todos.length === 0 && (
              <div className="note-empty">暂无待办，在下方输入后回车添加</div>
            )}
            {activePage.todos.map((t) => (
              <div key={t.id} className={`note-todo${t.done ? " done" : ""}`}>
                <button
                  className={`note-check${t.done ? " done" : ""}`}
                  onClick={() => toggleTodo(activePage.id, t.id)}
                  aria-label={t.done ? "标记为未完成" : "标记为完成"}
                >
                  {t.done && <CheckIcon />}
                </button>
                <input
                  className="note-todo-text"
                  value={t.text}
                  spellCheck={false}
                  onChange={(e) => updateTodoText(activePage.id, t.id, e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      // 编辑完成后回车跳到底部输入框继续快速添加
                      e.preventDefault();
                      addInputRef.current?.focus();
                    }
                  }}
                />
                <button
                  className="note-todo-x"
                  title="删除"
                  onClick={() => removeTodo(activePage.id, t.id)}
                >
                  <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                    <path
                      d="M2.5 2.5l7 7M9.5 2.5l-7 7"
                      stroke="currentColor"
                      strokeWidth="1.4"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              </div>
            ))}
          </div>
          <div className="note-todo-addwrap">
            <input
              ref={addInputRef}
              className="note-todo-add"
              value={draftTodo}
              placeholder="添加待办，回车保存"
              spellCheck={false}
              onChange={(e) => setDraftTodo(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addTodo();
                }
              }}
            />
          </div>
        </>
      )}
    </div>
  );
}
