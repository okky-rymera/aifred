// The "waiting for" list on the home screen: what you wait for from other
// people (waiting.rs). Under it, the emails that you sent and that got no
// answer yet (gmail.rs). "Track" puts one on the list. An email item closes
// by itself when the person answers. A check day makes AIfred tell you on
// that day when there is still no answer.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import { dayOfSecs, daysBetween, shortDay } from "./items";

/** How many email suggestions show before "Show all". */
const FIRST_SUGGESTIONS = 3;

/** Time with no typing before a new check day saves. */
const CHECK_DAY_WAIT_MS = 600;

/** True for a value that the date box can give: "" or "YYYY-MM-DD" with a real year. */
function finishedDay(v: string) {
  return v === "" || (/^\d{4}-\d{2}-\d{2}$/.test(v) && Number(v.slice(0, 4)) >= 1000);
}

function CheckDay({ w }: { w: api.Waiting }) {
  const { today, changeWaiting } = useApp();
  // While the user types, the box shows the typed day, and AIfred saves it
  // only after a short pause or when the box loses focus. Typing a year
  // gives days like 0002-11-06 first: these never save.
  const [draft, setDraft] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // One save at a time, in order, so an old day cannot finish last.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const saved = w.follow_up ?? "";
  const save = (v: string) => {
    clearTimeout(timer.current);
    timer.current = undefined;
    if (!finishedDay(v) || v === saved) {
      setDraft(null);
      return;
    }
    queue.current = queue.current.then(() => changeWaiting(w.id, { follow_up: v })).finally(() => setDraft((d) => (d === v ? null : d)));
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  const shown = draft ?? saved;
  const late = shown !== "" && shown <= today;
  return (
    <label className={`w-check${late ? " late" : ""}`} title="On this day, AIfred tells you when there is still no answer">
      <span>{shown ? `Check ${shortDay(shown, today)}` : "Check on…"}</span>
      <input
        type="date"
        aria-label={`Check day: ${w.what}`}
        value={shown}
        onChange={(e) => {
          const v = e.target.value;
          setDraft(v);
          clearTimeout(timer.current);
          timer.current = setTimeout(() => save(v), CHECK_DAY_WAIT_MS);
        }}
        onBlur={() => {
          if (timer.current !== undefined && draft !== null) save(draft);
        }}
      />
    </label>
  );
}

function WaitRow({ w }: { w: api.Waiting }) {
  const { today, changeWaiting, removeWaiting } = useApp();
  const days = daysBetween(w.since, today);
  if (w.done) {
    return (
      <li className="w-row done">
        <span className="w-main">
          <span className="w-line">
            <b>{w.who}</b> · {w.what}
          </span>
          <span className="w-why">{w.done_why}</span>
        </span>
        <button className="linkbtn" onClick={() => changeWaiting(w.id, { done: false })}>
          Open again
        </button>
      </li>
    );
  }
  return (
    <li className="w-row">
      <span className="w-main">
        <span className="w-line">
          <b>{w.who}</b> · {w.what}
        </span>
        <span className="w-since">{days <= 0 ? "since today" : `${days} day${days > 1 ? "s" : ""}, since ${shortDay(w.since, today)}`}</span>
      </span>
      <CheckDay w={w} />
      {w.url && (
        <a className="iconbtn" href={w.url} title="Open the email in Gmail">
          ↗
        </a>
      )}
      <button className="btn" onClick={() => changeWaiting(w.id, { done: true })} title="It came, or you stopped waiting">
        Got it
      </button>
      <button className="x" aria-label={`Delete: ${w.what}`} onClick={() => removeWaiting(w)}>
        ✕
      </button>
    </li>
  );
}

function AddForm({ onDone }: { onDone: () => void }) {
  const { addWaiting } = useApp();
  const [what, setWhat] = useState("");
  const [who, setWho] = useState("");
  const [day, setDay] = useState("");
  const busy = useRef(false);
  const add = async () => {
    if (busy.current || !what.trim() || !who.trim()) return;
    busy.current = true;
    try {
      if (await addWaiting({ what, who, follow_up: day || undefined })) onDone();
    } finally {
      busy.current = false;
    }
  };
  return (
    <form
      className="w-add"
      onSubmit={(e) => {
        e.preventDefault();
        add();
      }}
    >
      <input aria-label="What you wait for" placeholder="What, for example: the signed contract" maxLength={200} value={what} onChange={(e) => setWhat(e.target.value)} autoFocus />
      <input aria-label="Who you wait for" placeholder="Who" maxLength={200} value={who} onChange={(e) => setWho(e.target.value)} />
      <input type="date" aria-label="Check day" title="On this day, AIfred tells you when there is still no answer" value={day} onChange={(e) => setDay(e.target.value)} />
      <button className="btn" type="submit" disabled={!what.trim() || !who.trim()}>
        Add
      </button>
      <button className="linkbtn" type="button" onClick={onDone}>
        Cancel
      </button>
    </form>
  );
}

function Suggestions() {
  const { today, sentMail, trackEmail, hideSent } = useApp();
  const [all, setAll] = useState(false);
  if (!sentMail.length) return null;
  const shown = all ? sentMail : sentMail.slice(0, FIRST_SUGGESTIONS);
  return (
    <div className="w-sugg">
      <div className="w-sub">No answer yet to your emails</div>
      <ul className="w-list" aria-label="Emails with no answer yet">
        {shown.map((m) => (
          <li key={m.id} className="w-row">
            <span className="w-main">
              <a href={m.url} title="Open the email in Gmail">
                {m.subject}
              </a>
              <span className="w-since">
                to {m.people.join(", ")}
                {m.more_people ? ` +${m.more_people}` : ""} · sent {dayOfSecs(m.since, today)}
                {m.question ? " · has a question" : ""}
              </span>
            </span>
            <button className="btn" onClick={() => trackEmail(m)} title="Put it on the list. It closes by itself when they answer.">
              Track
            </button>
            <button className="linkbtn" onClick={() => hideSent(m)} title="You wait for nothing. It comes back when a new message comes.">
              Hide
            </button>
          </li>
        ))}
      </ul>
      {sentMail.length > FIRST_SUGGESTIONS && (
        <button className="linkbtn" onClick={() => setAll((a) => !a)}>
          {all ? "Show fewer" : `Show all (${sentMail.length})`}
        </button>
      )}
    </div>
  );
}

export function WaitingCard() {
  const { waiting, waitingError, sentMail, mailState } = useApp();
  const [adding, setAdding] = useState(false);
  const open = waiting.filter((w) => !w.done);
  const closed = waiting.filter((w) => w.done);
  const mailOn = mailState !== "off";
  // Nothing to show: only a small link, so the home screen stays short.
  if (!waiting.length && !sentMail.length && !adding && !waitingError) {
    return (
      <div className="w-empty">
        <button className="linkbtn" onClick={() => setAdding(true)}>
          + Waiting for someone?
        </button>
      </div>
    );
  }
  return (
    <section className="waiting" aria-label="Waiting for">
      <div className="eyebrow">
        <span>⏳ Waiting for{open.length ? ` · ${open.length}` : ""}</span>
        <span className="sp" />
        {!adding && (
          <button className="linkbtn" onClick={() => setAdding(true)}>
            Add
          </button>
        )}
      </div>
      {waitingError && <div className="note-line err">Cannot load the list: {waitingError}</div>}
      {adding && <AddForm onDone={() => setAdding(false)} />}
      {waiting.length > 0 && (
        <ul className="w-list" aria-label="Waiting for list">
          {open.map((w) => (
            <WaitRow key={w.id} w={w} />
          ))}
          {closed.map((w) => (
            <WaitRow key={w.id} w={w} />
          ))}
        </ul>
      )}
      {!waiting.length && !adding && <div className="goal-empty">{mailOn ? "Track an email below, or add what you wait for." : "Add what you wait for from other people."}</div>}
      <Suggestions />
    </section>
  );
}
