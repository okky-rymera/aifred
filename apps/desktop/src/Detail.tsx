// The details of one item, on the right side (or the full window when the
// window is narrow). Your own tasks can be changed here. GitHub and ClickUp
// items show what AIfred knows, with a link to open them there.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { Composer, Files, Md, Messages } from "./Chat";
import { useApp } from "./ctx";
import { SRC_NAME, dayOfSecs, dueInfo, fmtTime, shortDay, type Item } from "./items";
import { Minutes } from "./Plan";
import { PriPill } from "./Home";
import { SameWork } from "./Links";
import { NextStep, SuggestSteps } from "./Steps";
import { DocSuggest } from "./Docs";
import { SheetComments } from "./Sheets";
import { ReplyDraft, useItemActions } from "./Drafts";
import { ItemContextSection } from "./Context";

/** Text that saves when you leave the box, or press Enter. */
function useField(value: string, save: (v: string) => void) {
  const [v, setV] = useState(value);
  const last = useRef(value);
  // A reload with a new value (for example from AIfred) shows it, when you do not type.
  useEffect(() => {
    if (value !== last.current) {
      last.current = value;
      setV(value);
    }
  }, [value]);
  return {
    value: v,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setV(e.target.value),
    onBlur: () => {
      if (v !== last.current) {
        last.current = v;
        save(v);
      }
    },
  };
}

function LocalFields({ item }: { item: Item }) {
  const { editTask, today } = useApp();
  const t = item.task!;
  const tags = useField(t.tags.join(", "), (v) => editTask(t.id, { tags: v.split(",").map((x) => x.trim()).filter(Boolean) }));
  return (
    <>
      <dt>Due</dt>
      <dd>
        <input type="date" aria-label="Due date" value={t.due ?? ""} onChange={(e) => editTask(t.id, { due: e.target.value })} />
      </dd>
      <dt title="The task waits under Later until this day">Start</dt>
      <dd>
        <input type="date" aria-label="Start date" value={t.start ?? ""} onChange={(e) => editTask(t.id, { start: e.target.value })} />
      </dd>
      <dt>Priority</dt>
      <dd>
        <select aria-label="Priority" value={t.priority ?? ""} onChange={(e) => editTask(t.id, { priority: e.target.value as api.Priority | "" })}>
          <option value="">No priority</option>
          <option value="urgent">Urgent</option>
          <option value="high">High</option>
          <option value="normal">Normal</option>
          <option value="low">Low</option>
        </select>
      </dd>
      <dt>Tags</dt>
      <dd>
        <input
          type="text"
          aria-label="Tags"
          placeholder="With commas between them, for example work, travel"
          {...tags}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        />
      </dd>
      {t.due && (
        <>
          <dt />
          <dd className="hintx">{dueInfo(t.due, today).text}</dd>
        </>
      )}
    </>
  );
}

function Steps({ item }: { item: Item }) {
  const { tickStep, editTask } = useApp();
  const t = item.task!;
  const [step, setStep] = useState("");
  return (
    <>
      <div className="d-sec">
        Checklist{t.checklist.length > 0 && <span className="c">{t.checklist.filter((c) => c.done).length}/{t.checklist.length}</span>}
      </div>
      <ul className="steps">
        {t.checklist.map((c, i) => (
          <li key={`${i}-${c.text}`}>
            <button className="chk" aria-pressed={c.done} aria-label={`Step done: ${c.text}`} onClick={() => tickStep(t.id, i)} />
            <span className={`st${c.done ? " sd" : ""}`}>{c.text}</span>
            <button className="rm" aria-label={`Remove step: ${c.text}`} onClick={() => editTask(t.id, { checklist: t.checklist.filter((_, j) => j !== i) })}>
              ✕
            </button>
          </li>
        ))}
        <li>
          <input
            className="addstep"
            aria-label="New step"
            value={step}
            placeholder="+ Add a step, then press Enter"
            onChange={(e) => setStep(e.target.value)}
            onKeyDown={async (e) => {
              if (e.key === "Enter" && step.trim()) {
                const text = step.trim();
                setStep("");
                if (!(await editTask(t.id, { checklist: [...t.checklist, { text, done: false }] }))) setStep(text);
              }
            }}
          />
        </li>
      </ul>
    </>
  );
}

function Description({ item }: { item: Item }) {
  const { editTask, open, byKey } = useApp();
  const [extra, setExtra] = useState<api.ItemDetails | null>(null);
  const [error, setError] = useState("");
  const t = item.task;
  const notes = useField(t?.notes ?? "", (v) => t && editTask(t.id, { notes: v }));
  useEffect(() => {
    if (t) return;
    let live = true;
    setExtra(null);
    setError("");
    api
      .itemDetails(item.key)
      .then((d) => live && setExtra(d))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [item.key, t]);
  if (t) {
    return (
      <>
        <div className="d-sec">Description</div>
        <textarea className="d-text" aria-label="Task description" placeholder="Add notes for this task" {...notes} />
      </>
    );
  }
  return (
    <>
      <div className="d-sec">Description · from {SRC_NAME[item.src]}</div>
      <div className="desc">
        {error ? (
          <span className="d-err">{error}</span>
        ) : !extra ? (
          <span style={{ color: "var(--faint)" }}>Loading…</span>
        ) : extra.details.trim() ? (
          <Md text={extra.details} onItem={open} hasItem={(k) => byKey.has(k)} />
        ) : (
          <span style={{ color: "var(--faint)" }}>
            AIfred has no description of this item yet. It saves the description when it reads the item, for example when you ask about it below.
          </span>
        )}
      </div>
    </>
  );
}

function Links({ item }: { item: Item }) {
  const links = item.gh?.links ?? [];
  if (!links.length) return null;
  return (
    <>
      <div className="d-sec">
        Linked <span className="c">{links.length}</span>
      </div>
      <div className="links">
        {links.map((l) => (
          <a key={l.id} href={l.url} className="x" title={l.title}>
            {l.pull_request ? "PR" : "Issue"} {l.repo === item.gh!.repo ? "" : l.repo.split("/")[1]}#{l.number}
          </a>
        ))}
      </div>
    </>
  );
}

export function Detail(props: { item: Item; hidden?: boolean; tabs?: React.ReactNode; handle?: React.ReactNode }) {
  const { close, toggleDone, sched, addToPlan, removeFromPlan, plan, itemChat, askItem, models, setModel, open, byKey, deleteTask, today, editTask } = useApp();
  const i = props.item;
  const chat = itemChat(i.key);
  const actions = useItemActions(i.key);
  const body = useRef<HTMLDivElement>(null);
  const at = sched.at[i.key];
  const why = plan?.items.find((p) => p.key === i.key)?.why ?? i.why;
  const title = useField(i.title, (v) => {
    if (i.task && v.trim()) editTask(i.task.id, { title: v });
  });

  useEffect(() => {
    if (chat.msgs.length || chat.busy) body.current?.scrollTo({ top: body.current.scrollHeight, behavior: "smooth" });
  }, [chat.msgs.length, chat.busy]);

  return (
    <aside className="detail" role="dialog" aria-label="Task details" hidden={props.hidden}>
      {props.handle}
      {props.tabs}
      <div className="d-head">
        <button className="iconbtn back" onClick={close}>
          ← Back
        </button>
        <span className={`src ${i.src}`}>
          {SRC_NAME[i.src]}
          {i.ref && ` · ${i.ref}`}
        </span>
        <span className="sp" />
        {i.url && (
          <a className="iconbtn" href={i.url}>
            Open in {SRC_NAME[i.src]} ↗
          </a>
        )}
        {i.task && (
          <button className="iconbtn danger" onClick={() => deleteTask(i)} title="Delete this task">
            Delete
          </button>
        )}
        <button className="iconbtn close" onClick={close} aria-label="Close details">
          ✕
        </button>
      </div>
      <div className="d-body" ref={body}>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {i.task ? (
            <input className="d-title" aria-label="Task title" {...title} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          ) : (
            <h3 className="d-title">{i.title}</h3>
          )}
          <div className="d-status">
            {i.src !== "gh" ? (
              <button className={`statusbtn${i.done ? " isdone" : ""}`} onClick={() => toggleDone(i)}>
                {i.done ? "✓ Finished" : i.src === "cu" ? "Close in ClickUp" : i.src === "gm" ? "No answer needed" : "Mark as finished"}
              </button>
            ) : (
              <span className="hintx">GitHub is read-only. The item leaves the list when it closes there.</span>
            )}
            {i.isNew && <span className="pill">New today</span>}
            <PriPill p={i.priority} />
            {i.task?.added_by === "aifred" && <span className="pill high">Added by AIfred</span>}
          </div>
        </div>
        {i.summary && (
          <div className="tldr">
            <span className="k">✦ Summary by AIfred</span>
            {i.summary}
          </div>
        )}
        <dl className="fields">
          {i.task ? (
            <LocalFields item={i} />
          ) : (
            <>
              {i.mail ? (
                <>
                  <dt>Waits since</dt>
                  <dd className="ro">
                    {dayOfSecs(i.mail.since, today)} · {i.mail.messages} message{i.mail.messages > 1 ? "s" : ""}
                    {i.mail.direct ? "" : " · you are in Cc"}
                  </dd>
                </>
              ) : (
                <>
                  <dt>Due</dt>
                  <dd className="ro">{i.due ? `${shortDay(i.due, today)} · ${dueInfo(i.due, today).text}` : "No due day"}</dd>
                </>
              )}
              {i.cu && (
                <>
                  <dt>Status</dt>
                  <dd className="ro">{i.cu.status}</dd>
                </>
              )}
            </>
          )}
          <dt>Time guess</dt>
          <dd>
            <span className="ctl">
              <Minutes item={i} />
              <span className="hintx">{i.minutesBy === "aifred" ? "✦ guessed by AIfred" : i.minutesBy === "you" ? "set by you" : "no guess yet"}</span>
            </span>
          </dd>
          <dt>Today's plan</dt>
          <dd>
            {at !== undefined ? (
              <>
                <span className="planned">
                  {fmtTime(at)}–{fmtTime(at + i.minutes)}
                </span>{" "}
                <button className="linkbtn" onClick={() => removeFromPlan(i.key)}>
                  Take out
                </button>
              </>
            ) : (
              <button className="linkbtn" style={{ paddingLeft: 6 }} disabled={i.done} onClick={() => addToPlan(i.key)}>
                + Add to today's plan
              </button>
            )}
          </dd>
        </dl>
        {why && !i.done && <span className="why">{why}</span>}
        <NextStep item={i} />
        <Description item={i} />
        <ItemContextSection key={i.key} itemKey={i.key} version={chat.busy ? -1 : chat.msgs.length} />
        <Links item={i} />
        <SameWork item={i} />
        {i.task && <Steps item={i} />}
        {!i.task && !i.done && <div className="d-sec">Steps</div>}
        <SuggestSteps item={i} />
        <ReplyDraft item={i} log={actions.list.filter((a) => a.app === "gmail")} load={actions.reload} />
        <DocSuggest item={i} log={actions.list.filter((a) => a.app === "docs")} load={actions.reload} />
        <SheetComments item={i} log={actions.list.filter((a) => a.app === "sheets")} load={actions.reload} />
        {i.task && i.task.files.length > 0 && (
          <>
            <div className="d-sec">
              Files <span className="c">{i.task.files.length}</span>
            </div>
            <Files files={i.task.files} />
          </>
        )}
        {(chat.msgs.length > 0 || chat.busy) && (
          <>
            <div className="d-sec">Asked about this item</div>
            <div className="d-asks">
              <Messages list={chat.msgs} busy={chat.busy} progress={chat.progress} onItem={open} hasItem={(k) => byKey.has(k)} />
            </div>
          </>
        )}
      </div>
      <div className="d-ask">
        <Composer
          id={`item-${i.key}`}
          placeholder="Ask AIfred about this item"
          sendLabel="Ask"
          busy={chat.busy}
          model={models.item}
          onModel={(m) => setModel("item", m)}
          onSend={(text, files) => askItem(i.key, text, files)}
        />
      </div>
    </aside>
  );
}
