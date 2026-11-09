// One mission of today on the right side, like the details of an item:
// its items, its checklist, the actions, and a chat about the mission.
// The chat is the side chat "mission:<id>" (side.rs). It keeps its
// messages when the brief keeps the mission's id.
import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { Composer, Messages } from "./Chat";
import { useApp } from "./ctx";
import { srcText } from "./Links";
import { NextStep } from "./Steps";
import { GoalTag } from "./Goals";
import { parentOf, type Item } from "./items";

/** The key of a mission's chat in the list of item chats. Item keys never start with "mission:". */
export const missionChatKey = (id: number) => `mission:${id}`;
/** The side chat name (side.rs) of an item key or a mission chat key. */
export const sideThread = (key: string) => (key.startsWith("mission:") ? key : `item:${key}`);

/** The text that says how AIfred got the progress of a mission. */
export function progressText(p: api.MissionProgress): string {
  return `${p.steps_done} of ${p.steps} steps done and ${p.items_closed} of ${p.items} items closed: (${p.steps_done} + ${p.items_closed}) / (${p.steps} + ${p.items}) = ${p.percent}%`;
}

/** Why an item of a mission can or cannot close from the "Done" question. */
function closeTip(i: Item | undefined): string {
  if (!i) return "This item is not in your list now.";
  if (i.gh) return "GitHub is read-only. The item leaves the list when it closes on GitHub.";
  if (i.cu) return "Close this task in ClickUp";
  if (i.mail) return "No answer needed: hide this email until a new message comes. Gmail does not change.";
  return "Finish this task";
}

/** "Done" on a mission: you pick the items that are finished too. */
export function FinishMission({ m, onCancel }: { m: api.Mission; onCancel: () => void }) {
  const { byKey, finishMission } = useApp();
  const open = m.items.filter((i) => i.open);
  const canClose = (k: string) => {
    const i = byKey.get(k);
    return !!i && !i.gh;
  };
  const [pick, setPick] = useState(() => new Set(open.filter((i) => canClose(i.key)).map((i) => i.key)));
  const [busy, setBusy] = useState(false);
  const flip = (k: string) =>
    setPick((s) => {
      const n = new Set(s);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });
  return (
    <div className="m-panel" role="group" aria-label={`Finish mission: ${m.title}`}>
      <div className="d-sec">Which items are finished too?</div>
      {open.length === 0 ? (
        <p className="m-note">All items of this mission are closed.</p>
      ) : (
        <ul className="m-items">
          {open.map((i) => (
            <li key={i.key}>
              <label title={closeTip(byKey.get(i.key))}>
                <input type="checkbox" disabled={!canClose(i.key)} checked={pick.has(i.key)} onChange={() => flip(i.key)} /> {i.title}
              </label>
            </li>
          ))}
        </ul>
      )}
      <div className="m-acts">
        <button
          className="btn pri"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            if (!(await finishMission(m, [...pick]))) setBusy(false);
          }}
        >
          Finish mission
        </button>
        <button className="btn" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

/** The panel of an open mission: its items, its checklist, how AIfred got
 * the progress, and the actions. A tick in the checklist does not change the items. */
export function MissionPanel({ m, onDone }: { m: api.Mission; onDone: () => void }) {
  const { byKey, items, open, startWork, work, missionStep, correctMission, addToMission, removeFromMission } = useApp();
  const [busy, setBusy] = useState(false);
  // The last open item cannot go out: for that, use Done or Not today.
  const openCount = m.items.filter((i) => i.open).length;
  // The open items that you can add: all open items that are not in this mission.
  const addable = useMemo(() => {
    const inIt = new Set(m.items.map((i) => i.key));
    return items.filter((i) => !i.done && !inIt.has(i.key)).sort((a, b) => a.title.localeCompare(b.title));
  }, [items, m.items]);
  const act = (f: () => Promise<boolean>) => {
    setBusy(true);
    void f().finally(() => setBusy(false));
  };
  // Start works on the item of the next action, or else on the first open item.
  const first = m.items.find((i) => i.open);
  const working = !!work && m.items.some((i) => i.key === work.key);
  const p = m.progress;
  return (
    <div className="m-panel" role="group" aria-label={`Mission: ${m.title}`}>
      <div className="d-sec">
        Items <span className="c">{p.items_closed}/{p.items} closed</span>
      </div>
      <ul className="m-items">
        {m.items.map((i) => {
          const it = byKey.get(i.key);
          return (
            <li key={i.key} className={i.open ? "" : "sd"}>
              {it ? (
                <button className="linkbtn" aria-label={`Open details: ${i.title}`} onClick={() => open(i.key)}>
                  {i.title}
                </button>
              ) : (
                <span>{i.title}</span>
              )}
              {it && <span className="m-src">{srcText(it)}</span>}
              {/* AIfred cannot see why an outside item left your list: it is closed there, or it is not yours now. */}
              {!i.open && <span className="m-src">{i.key.startsWith("task:") ? "Closed" : "Left your list"}</span>}
              {i.open && i.by === "you" && <span className="m-src">You added it</span>}
              {i.open && openCount > 1 && (
                <button
                  className="linkbtn m-out"
                  disabled={busy}
                  aria-label={`Take out of the mission: ${i.title}`}
                  title="This item is not part of this work. AIfred does not put it back in this mission."
                  onClick={() => act(() => removeFromMission(m, i.key))}
                >
                  Take out
                </button>
              )}
            </li>
          );
        })}
      </ul>
      {addable.length > 0 && (
        <select
          className="m-add"
          aria-label={`Add an item to the mission: ${m.title}`}
          title="The item leaves its other mission. AIfred keeps it in this one."
          disabled={busy}
          value=""
          onChange={(e) => {
            const key = e.target.value;
            if (key) act(() => addToMission(m, key));
          }}
        >
          <option value="">Add an item…</option>
          {addable.map((i) => (
            <option key={i.key} value={i.key}>
              {i.title}
            </option>
          ))}
        </select>
      )}
      {m.checklist.length > 0 && (
        <>
          <div className="d-sec">
            Checklist <span className="c">{p.steps_done}/{p.steps}</span>
          </div>
          <ul className="steps m-steps">
            {m.checklist.map((c, n) => (
              <li key={`${n}-${c.text}`}>
                <button className="chk" aria-pressed={c.done} aria-label={`Step done: ${c.text}`} onClick={() => missionStep(m, n)} />
                <span className={`st${c.done ? " sd" : ""}`}>{c.text}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      <p className="m-note">Progress: {progressText(p)}. A tick in the checklist does not close an item.</p>
      <div className="m-acts">
        {first && !working && (
          <button className="btn" title="Start work on this. AIfred keeps the real time, to make better time guesses." onClick={() => startWork(first.key)}>
            Start
          </button>
        )}
        <button className="btn pri" onClick={onDone}>
          Done
        </button>
        <span className="sp" />
        {/* A correction is for open items. With none left, finish the mission. */}
        {first && (
          <>
            <button
              className="linkbtn"
              aria-label={`Not today: ${m.title}`}
              title="This does not need you today. The brief stops counting it for today."
              onClick={() => correctMission(m, "not_today")}
            >
              Not today
            </button>
            <button
              className="linkbtn"
              aria-label={`Wrong priority: ${m.title}`}
              title="The brief ranked this too high. AIfred ranks its items lower from now on."
              onClick={() => correctMission(m, "wrong_priority")}
            >
              Wrong priority
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/** A mission on the right side. "Done" shows the question about the items in the same place. */
export function MissionDetail(props: { m: api.Mission; hidden?: boolean; tabs?: React.ReactNode; handle?: React.ReactNode }) {
  const { close, byKey, links, open, itemChat, askItem, models, setModel } = useApp();
  const m = props.m;
  const [finishing, setFinishing] = useState(false);
  const key = missionChatKey(m.id);
  const chat = itemChat(key);
  const body = useRef<HTMLDivElement>(null);
  // The first open item has the next action, as on the mission card.
  const leadKey = m.items.find((i) => i.open)?.key;
  const lead = leadKey ? byKey.get(leadKey) : undefined;
  const parentKey = leadKey ? parentOf(links.links, leadKey) : undefined;
  const parent = parentKey && !m.items.some((i) => i.key === parentKey) ? byKey.get(parentKey) : undefined;

  useEffect(() => {
    if (chat.msgs.length || chat.busy) body.current?.scrollTo({ top: body.current.scrollHeight, behavior: "smooth" });
  }, [chat.msgs.length, chat.busy]);

  return (
    <aside className="detail" role="dialog" aria-label="Mission details" hidden={props.hidden}>
      {props.handle}
      {props.tabs}
      <div className="d-head">
        <button className="iconbtn back" onClick={close}>
          ← Back
        </button>
        <span className="src">Mission · {m.progress.percent}% done</span>
        <span className="sp" />
        <button className="iconbtn close" onClick={close} aria-label="Close the mission">
          ✕
        </button>
      </div>
      <div className="d-body" ref={body}>
        <h3 className="d-title">{m.title}</h3>
        {m.doer === "aifred" && <span className="tagc">AIfred does the next step</span>}
        {parent && (
          <div className="part-of">
            Part of{" "}
            <button className="linkbtn" aria-label={`Open details: ${parent.title}`} onClick={() => open(parent.key)}>
              {parent.title}
            </button>
          </div>
        )}
        {lead?.why && (
          <span className="why">
            {lead.why} <GoalTag goal={lead.goal} ranked />
          </span>
        )}
        {lead && <NextStep item={lead} />}
        {finishing ? <FinishMission m={m} onCancel={() => setFinishing(false)} /> : <MissionPanel m={m} onDone={() => setFinishing(true)} />}
        {(chat.msgs.length > 0 || chat.busy) && (
          <>
            <div className="d-sec">Asked about this mission</div>
            <div className="d-asks">
              <Messages list={chat.msgs} busy={chat.busy} progress={chat.progress} onItem={open} hasItem={(k) => byKey.has(k)} />
            </div>
          </>
        )}
      </div>
      <div className="d-ask">
        <Composer
          id={key}
          placeholder="Ask AIfred about this mission"
          sendLabel="Ask"
          busy={chat.busy}
          model={models.item}
          onModel={(mo) => setModel("item", mo)}
          onSend={(text, files) => askItem(key, text, files)}
        />
      </div>
    </aside>
  );
}
