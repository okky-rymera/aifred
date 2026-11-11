// The parts that each chat uses: the chat box (text, files, model, send),
// the message list, and Markdown where a link to "item:<key>" opens the item.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import Markdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import * as api from "./api";
import { kb } from "./items";

export const ICON = {
  home: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M3 11l9-7 9 7v9a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1z" /></svg>,
  chat: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.4A8 8 0 1 1 21 12z" /></svg>,
  mem: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 3a6 6 0 0 0-6 6c0 2.2 1.2 3.6 2.4 4.8.8.8 1.1 1.7 1.1 2.7V18h5v-1.5c0-1 .3-1.9 1.1-2.7C16.8 12.6 18 11.2 18 9a6 6 0 0 0-6-6zM10 21h4" /></svg>,
  gear: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-2.9 1.2V21a2 2 0 1 1-4 0v-.1A1.7 1.7 0 0 0 7 19.4l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A1.7 1.7 0 0 0 3 13.7H3a2 2 0 1 1 0-4h.1A1.7 1.7 0 0 0 4.6 7l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A1.7 1.7 0 0 0 10 3V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 2.9 1.2l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0 1.2 2.9H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></svg>,
  cal: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M3 10h18M8 3v4M16 3v4" /></svg>,
  spark: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round"><path d="M12 3l2 6 6 2-6 2-2 6-2-6-6-2 6-2z" /></svg>,
  clip: <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M21 11.5l-8.6 8.6a5.5 5.5 0 0 1-7.8-7.8l8.6-8.6a3.7 3.7 0 0 1 5.2 5.2l-8.6 8.6a1.8 1.8 0 0 1-2.6-2.6l7.9-7.9" /></svg>,
  task: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="4" y="4" width="16" height="16" rx="3" /><path d="M8.5 12l2.5 2.5 4.5-5" /></svg>,
  clock: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>,
  hand: <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" /></svg>,
};

/** Keeps "item:" links. The default drops links with an unknown scheme. */
const keepItemLinks = (url: string) => (url.startsWith("item:") ? url : defaultUrlTransform(url));

/** Markdown. A link to "item:<key>" is a button that opens the item, when the item is in the list. */
export function Md(props: { text: string; onItem?: (key: string) => void; hasItem?: (key: string) => boolean; className?: string }) {
  return (
    <div className={`md ${props.className ?? ""}`}>
      <Markdown
        remarkPlugins={[remarkGfm]}
        urlTransform={keepItemLinks}
        components={{
          a: ({ href, children }) => {
            if (href?.startsWith("item:")) {
              const key = href.slice(5);
              if (props.onItem && (props.hasItem?.(key) ?? true)) {
                return (
                  <button className="ref" onClick={() => props.onItem!(key)}>
                    {children}
                  </button>
                );
              }
              return <span>{children}</span>;
            }
            return <a href={href}>{children}</a>;
          },
        }}
      >
        {props.text}
      </Markdown>
    </div>
  );
}

export type ChatMsg = {
  role: "user" | "assistant" | "error" | "intro";
  text: string;
  files?: api.FileRef[];
  /** The time, model, lookups, and cost line under an answer. */
  info?: string;
  /** Facts from memory that the app added to the question. */
  facts?: string[];
  /** Summaries of earlier days that the app added to the question. */
  summaries?: string[];
};

/** "1 fact and 2 summaries", for the line under an answer. */
export function usedLine(facts: number, summaries: number): string {
  const parts: string[] = [];
  if (facts > 0) parts.push(`${facts} fact${facts === 1 ? "" : "s"}`);
  if (summaries > 0) parts.push(`${summaries} ${summaries === 1 ? "summary" : "summaries"}`);
  return parts.join(" and ");
}

export function Files(props: { files: api.FileRef[]; onRemove?: (i: number) => void; busy?: string[] }) {
  if (!props.files.length && !props.busy?.length) return null;
  return (
    <div className="atts">
      {props.files.map((f, i) =>
        props.onRemove ? (
          <span key={f.id} className="att" title={f.name}>
            <span className="nm">{f.name}</span>
            <span className="sz">{kb(f.size)}</span>
            <button onClick={() => props.onRemove!(i)} aria-label={`Remove ${f.name}`}>
              ✕
            </button>
          </span>
        ) : (
          <button key={f.id} className="att" title={`Open ${f.name}`} aria-label={`Open ${f.name}`} onClick={() => api.openFile(f.id).catch(() => {})}>
            <span className="nm">{f.name}</span>
            <span className="sz">{kb(f.size)}</span>
          </button>
        ),
      )}
      {props.busy?.map((n, i) => (
        <span key={`busy-${i}`} className="att busy" title="Saving the file">
          <span className="nm">{n}</span>
          <span className="sz">…</span>
        </span>
      ))}
    </div>
  );
}

export function Messages(props: {
  list: ChatMsg[];
  busy?: boolean;
  progress?: string[];
  onItem?: (key: string) => void;
  hasItem?: (key: string) => boolean;
  busyText?: string;
}) {
  return (
    <>
      {props.list.map((m, i) =>
        m.role === "user" ? (
          <div key={i} className="m-user">
            {m.text}
            {m.files && m.files.length > 0 && <Files files={m.files} />}
          </div>
        ) : m.role === "error" ? (
          <div key={i} className="m-err" role="alert">
            {m.text}
          </div>
        ) : (
          <div key={i} className={`m-ai${m.role === "intro" ? " m-intro" : ""}`}>
            <Md text={m.text} onItem={props.onItem} hasItem={props.hasItem} />
            {m.info && <span className="mmeta">{m.info}</span>}
            {((m.facts?.length ?? 0) > 0 || (m.summaries?.length ?? 0) > 0) && (
              <details>
                <summary>
                  Used {usedLine(m.facts?.length ?? 0, m.summaries?.length ?? 0)} from memory
                </summary>
                <ul>
                  {[...(m.facts ?? []), ...(m.summaries ?? [])].map((f, j) => (
                    <li key={j}>{f}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        ),
      )}
      {props.busy && (
        <div className="typing">
          {(props.progress ?? []).map((p, i) => (
            <span key={i}>✓ {p}</span>
          ))}
          <span className="now">{props.busyText ?? "AIfred is thinking…"}</span>
        </div>
      )}
    </>
  );
}

/** The text and files in each chat box. They stay when the box closes and opens again. */
const drafts = new Map<string, { text: string; files: api.FileRef[] }>();

export function ModelPicker(props: { model: string | null; onChange: (model: string | null) => void; disabled?: boolean }) {
  return (
    <select
      aria-label="Model"
      title={api.CHAT_MODELS.find((m) => m.id === props.model)?.hint}
      value={props.model ?? ""}
      disabled={props.disabled}
      onChange={(e) => props.onChange(e.target.value || null)}
    >
      {api.CHAT_MODELS.map((m) => (
        <option key={m.id ?? ""} value={m.id ?? ""} title={m.hint}>
          {m.label}
        </option>
      ))}
    </select>
  );
}

/** One chat box: files, a text box that grows, then a row with the file
 * button, `hint` (what the app understood), the model, and Send. */
export function Composer(props: {
  id: string;
  placeholder: string;
  /** The name of the text box for screen readers. Default: the placeholder. */
  label?: string;
  /** Tab in the box calls this, and does not move the focus. Shift+Tab
   * still moves it, and so does Tab right after Esc. */
  onTab?: () => void;
  sendLabel?: string;
  /** True turns Send off. A function decides from the text, for example
   * to let the home box add a task while AIfred answers a question. */
  busy?: boolean | ((text: string, files: api.FileRef[]) => boolean);
  /** Returns false to keep the text and files, for example when the send failed. */
  onSend: (text: string, files: api.FileRef[]) => Promise<boolean | void> | boolean | void;
  /** The model picker. Leave out onModel for a box with no picker. */
  model?: string | null;
  onModel?: (model: string | null) => void;
  /** "task": the box adds a task. It gets a green border. */
  tone?: "task";
  suggestions?: string[];
  hint?: (text: string, files: api.FileRef[]) => React.ReactNode;
  autoFocus?: boolean;
  /** A text to put in the box, for example from a suggestion. A new object each time. */
  fill?: { text: string } | null;
}) {
  const saved = drafts.get(props.id);
  const [text, setText] = useState(saved?.text ?? "");
  const [files, setFiles] = useState<api.FileRef[]>(saved?.files ?? []);
  const [uploading, setUploading] = useState<string[]>([]);
  const [drop, setDrop] = useState(false);
  const [error, setError] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  /** True right after Esc: the next Tab moves the focus (see onTab). */
  const tabFree = useRef(false);
  const busy = typeof props.busy === "function" ? props.busy(text, files) : !!props.busy;

  useEffect(() => {
    drafts.set(props.id, { text, files });
  }, [props.id, text, files]);

  // Grows with the text, up to 7 lines. Then it scrolls.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    const max = 168;
    el.style.height = `${Math.min(el.scrollHeight, max)}px`;
    el.classList.toggle("full", el.scrollHeight > max);
  }, [text]);

  useEffect(() => {
    if (!props.fill) return;
    setText(props.fill.text);
    box.current?.focus();
  }, [props.fill]);

  async function add(list: FileList | File[]) {
    const all = [...list];
    if (!all.length) return;
    setError("");
    setUploading((u) => [...u, ...all.map((f) => f.name)]);
    for (const f of all) {
      try {
        const ref = await api.uploadFile(f);
        setFiles((l) => [...l, ref]);
      } catch (e) {
        setError(`Cannot attach ${f.name}: ${e}`);
      } finally {
        setUploading((u) => {
          const i = u.indexOf(f.name);
          return i < 0 ? u : [...u.slice(0, i), ...u.slice(i + 1)];
        });
      }
    }
    box.current?.focus();
  }

  async function send() {
    if (busy || uploading.length || (!text.trim() && !files.length)) return;
    const sent = text;
    const sentFiles = files;
    // Clear at once, so a fast second Enter does not send twice. Clear the
    // saved draft too: a send can change the page ("plan my day") before the
    // box saves its empty text.
    setText("");
    setFiles([]);
    drafts.set(props.id, { text: "", files: [] });
    const ok = await props.onSend(sent, sentFiles);
    if (ok === false) {
      setText((t) => t || sent);
      setFiles((f) => (f.length ? f : sentFiles));
    }
  }

  return (
    <div className="cmp-wrap">
      {props.suggestions && props.suggestions.length > 0 && (
        <div className="sugs">
          {props.suggestions.map((s) => (
            <button key={s} className="sug" onClick={() => (setText(s), box.current?.focus())}>
              {s}
            </button>
          ))}
        </div>
      )}
      <div
        className={`composer${drop ? " drop" : ""}${props.tone ? ` ${props.tone}` : ""}`}
        onDragOver={(e) => {
          if (![...e.dataTransfer.types].includes("Files")) return;
          e.preventDefault();
          setDrop(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setDrop(false);
        }}
        onDrop={(e) => {
          if (!e.dataTransfer.files.length) return;
          e.preventDefault();
          setDrop(false);
          add(e.dataTransfer.files);
        }}
      >
        <Files files={files} busy={uploading} onRemove={(i) => setFiles((l) => l.filter((_, j) => j !== i))} />
        <textarea
          ref={box}
          className="c-in"
          rows={1}
          value={text}
          autoFocus={props.autoFocus}
          aria-label={props.label ?? props.placeholder}
          placeholder={props.placeholder}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => {
            if (e.clipboardData.files.length) {
              e.preventDefault();
              add(e.clipboardData.files);
            }
          }}
          aria-keyshortcuts={props.onTab ? "Tab" : undefined}
          aria-description={props.onTab ? "Tab changes the mode. Press Escape, then Tab, to go to the next control." : undefined}
          onKeyDown={(e) => {
            // Esc frees the next Tab: keyboard users can leave the box forward.
            const free = tabFree.current;
            tabFree.current = e.key === "Escape";
            if (e.key === "Tab" && props.onTab && !free && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
              e.preventDefault();
              props.onTab();
              return;
            }
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              send();
            }
          }}
        />
        <div className="c-foot">
          <label className="attach" title="Attach files">
            {ICON.clip}
            <input
              type="file"
              multiple
              aria-label="Attach files"
              onChange={(e) => {
                if (e.target.files) add(e.target.files);
                e.target.value = "";
              }}
            />
          </label>
          <div className="parse">{props.hint?.(text, files)}</div>
          {props.onModel && <ModelPicker model={props.model ?? null} onChange={props.onModel} disabled={busy} />}
          <button className="send" onClick={send} disabled={busy || uploading.length > 0 || (!text.trim() && !files.length)}>
            {props.sendLabel ?? "Send"}
          </button>
        </div>
      </div>
      {error && <div className="d-err">{error}</div>}
    </div>
  );
}
