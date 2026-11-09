// The reviews on the home screen (review.rs). They need no AI call.
// - The end-of-day check: from 30 minutes before the work day ends. It
//   shows what you finished today and what is still open. You tick what
//   moves to tomorrow's plan.
// - The weekly review: every 7 days, first on a Friday. It asks if your
//   goals are still right. Then, for each item with no change for 3 weeks,
//   you pick one: do, plan, give to someone, or drop.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import { GoalEditor } from "./Goals";
import { addDays, dur, shortDay, workDay } from "./items";

/** The end-of-day check shows this many minutes before the work day ends. */
export const DAY_CHECK_BEFORE_END = 30;

/** Minutes after midnight now. Changes once a minute, so a card can show at its time. */
function useMinuteOfDay() {
  const now = () => {
    const d = new Date();
    return d.getHours() * 60 + d.getMinutes();
  };
  const [m, setM] = useState(now);
  useEffect(() => {
    const t = setInterval(() => setM(now()), 60_000);
    return () => clearInterval(t);
  }, []);
  return m;
}

function Place({ place }: { place: string }) {
  return place ? <span className="rv-place">{place}</span> : null;
}

/** The end-of-day check. `force`: show it now, also before its time (the "End the day" button). */
export function DayCheckCard({ force, onClose }: { force: boolean; onClose: () => void }) {
  const { today, settings, items, plan, work, toast, reload, open } = useApp();
  const minute = useMinuteOfDay();
  const time = minute >= workDay(settings)[1] - DAY_CHECK_BEFORE_END;
  const [check, setCheck] = useState<api.DayCheck | null>(null);
  const [error, setError] = useState("");
  // The items that you do not want to move. All others move.
  const [stay, setStay] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const want = force || time;

  // Load again when a part that the card shows changes, so a finished item
  // moves to "Done today". A refresh of GitHub or ClickUp with no such
  // change does not load it again.
  const shown = useMemo(
    () =>
      want
        ? [items.map((i) => `${i.key}:${i.done ? 1 : 0}:${i.due ?? ""}`).join("|"), plan?.items.map((p) => p.key).join("|") ?? "", work?.key ?? ""].join("#")
        : "",
    [want, items, plan, work],
  );
  useEffect(() => {
    if (!want) return;
    let live = true;
    api
      .dayCheck(today)
      .then((c) => live && (setCheck(c), setError("")))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [want, today, shown]);

  if (!want || !check || check.day !== today) return error && want ? <div className="note-line err">Cannot load the end-of-day check: {error}</div> : null;
  // A finished day shows again only with the button.
  if (check.finished && !force) return null;
  if (!force && !check.done.length && !check.left.length) return null;

  const finish = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const moved = check.left.map((i) => i.key).filter((k) => !stay.has(k));
      const c = await api.finishDay(today, moved);
      setCheck(c);
      reload();
      onClose();
      toast(moved.length ? `Day finished. ${moved.length} item${moved.length > 1 ? "s are" : " is"} in tomorrow's plan.` : "Day finished. Nothing moves to tomorrow.");
    } catch (e) {
      toast(`The end-of-day check is not saved: ${e}`, undefined, true);
    } finally {
      setBusy(false);
    }
  };
  const total = check.done.reduce((n, i) => n + (i.minutes ?? 0), 0);
  return (
    <section className="review" aria-label="End of the day">
      <div className="eyebrow">
        <span>☾ End of the day</span>
        <span className="sp" />
        {force && (
          <button className="linkbtn" onClick={onClose}>
            Close
          </button>
        )}
      </div>
      <div className="rv-h">
        Done today <span className="c">{check.done.length}</span>
        {total > 0 && <span className="rv-sub">· {dur(total)} on the timer</span>}
      </div>
      {check.done.length ? (
        <ul className="rv-list">
          {check.done.map((i) => (
            <li key={i.key} className="rv-row done">
              <button className="linkbtn rv-title" onClick={() => open(i.key)}>
                {i.title}
              </button>
              <Place place={i.place} />
              {i.minutes ? <span className="rv-sub">{dur(i.minutes)}</span> : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="rv-empty">Nothing finished today.</div>
      )}
      {check.left.length > 0 && (
        <>
          <div className="rv-h">
            Still open <span className="c">{check.left.length}</span>
            <span className="rv-sub">· the ticked items go into tomorrow's plan</span>
          </div>
          <ul className="rv-list">
            {check.left.map((i) => (
              <li key={i.key} className="rv-row">
                <label className="rv-move">
                  <input
                    type="checkbox"
                    aria-label={`Move to tomorrow: ${i.title}`}
                    checked={!stay.has(i.key)}
                    onChange={(e) =>
                      setStay((s) => {
                        const n = new Set(s);
                        if (e.target.checked) n.delete(i.key);
                        else n.add(i.key);
                        return n;
                      })
                    }
                  />
                  <span className="rv-title">{i.title}</span>
                </label>
                <Place place={i.place} />
              </li>
            ))}
          </ul>
        </>
      )}
      {check.finished && <div className="rv-empty">You finished this day already. A new finish moves the ticked items too.</div>}
      <div className="rv-acts">
        <button className="btn pri" disabled={busy} onClick={finish}>
          Finish the day
        </button>
      </div>
    </section>
  );
}

const PICK_DONE: Record<api.PickKind, (p: api.ReviewPick, today: string) => string> = {
  do: (p) => `In today's plan: ${p.title}`,
  plan: (p, today) => `Planned for ${shortDay(p.note, today)}: ${p.title}`,
  delegate: (p) => `Waiting for ${p.note}: ${p.title}`,
  drop: (p) => (p.key.startsWith("task:") ? `Dropped: ${p.title}` : `AIfred stops counting it: ${p.title}`),
};

function StaleRow({ item: i, onPick }: { item: api.StaleItem; onPick: (i: api.StaleItem, pick: api.PickKind, note?: string) => Promise<boolean> }) {
  const { today, open } = useApp();
  // The open form of the row: a day for "plan", a person for "delegate".
  const [form, setForm] = useState<"plan" | "delegate" | null>(null);
  const [note, setNote] = useState("");
  const local = i.key.startsWith("task:");
  const weeks = Math.floor(i.days / 7);
  const send = async () => {
    if (form && note.trim() && (await onPick(i, form, note))) setForm(null);
  };
  return (
    <li className="rv-row stale">
      <span className="rv-main">
        <button className="linkbtn rv-title" onClick={() => open(i.key)}>
          {i.title}
        </button>
        <span className="rv-meta">
          <Place place={i.place} />
          <span className="rv-sub">No change for {weeks} weeks</span>
        </span>
      </span>
      {form ? (
        <span className="rv-form">
          <input
            aria-label={form === "plan" ? `Day for: ${i.title}` : `Who does it: ${i.title}`}
            type={form === "plan" ? "date" : "text"}
            min={form === "plan" ? addDays(today, 1) : undefined}
            placeholder={form === "delegate" ? "Who? For example: Ana" : undefined}
            maxLength={200}
            value={note}
            autoFocus
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") send();
              if (e.key === "Escape") setForm(null);
            }}
          />
          <button className="btn pri" disabled={!note.trim()} onClick={send}>
            OK
          </button>
          <button className="linkbtn" onClick={() => setForm(null)}>
            Cancel
          </button>
        </span>
      ) : (
        <span className="rv-picks">
          <button className="btn" title="Put it in today's plan" onClick={() => onPick(i, "do")}>
            Do
          </button>
          <button
            className="btn"
            title={local ? "Pick a day. The task waits until then, and goes into that day's plan." : "Pick a day. It goes into that day's plan."}
            onClick={() => {
              setNote(addDays(today, 1));
              setForm("plan");
            }}
          >
            Plan
          </button>
          <button
            className="btn"
            title={local ? "Someone else does it. It goes on the waiting-for list, and the task closes." : "Someone else does it. It goes on the waiting-for list."}
            onClick={() => {
              setNote("");
              setForm("delegate");
            }}
          >
            Give to…
          </button>
          <button
            className="btn"
            title={local ? "You will not do it. The task closes." : "You will not do it. AIfred stops counting it in the brief and the reviews. Nothing changes in GitHub or ClickUp."}
            onClick={() => onPick(i, "drop")}
          >
            Drop
          </button>
        </span>
      )}
    </li>
  );
}

/** The weekly review. `force`: show it now, also when it is not due (the "Weekly review" button). */
export function WeekReviewCard({ force, onClose }: { force: boolean; onClose: () => void }) {
  const { today, goals, changeGoals, toast, reload } = useApp();
  const [review, setReview] = useState<api.WeekReview | null>(null);
  const [error, setError] = useState("");
  const [editGoals, setEditGoals] = useState(false);
  const [busy, setBusy] = useState(false);
  // One pick at a time, so two fast clicks do not pick twice.
  const picking = useRef(false);

  const load = useCallback(() => {
    api
      .weekReview(today)
      .then((r) => (setReview(r), setError("")))
      .catch((e) => setError(String(e)));
  }, [today]);
  useEffect(load, [load, goals]);

  if (error) return force ? <div className="note-line err">Cannot load the weekly review: {error}</div> : null;
  if (!review || (!review.due && !force)) return null;

  const onPick = async (i: api.StaleItem, pick: api.PickKind, note = ""): Promise<boolean> => {
    if (picking.current) return false;
    picking.current = true;
    try {
      const p = await api.reviewPick(i.key, pick, note.trim(), today);
      // With more items than the card shows, load again: the next item comes in, and the counts are right.
      if (review.more > 0) load();
      else setReview((r) => r && { ...r, items: r.items.filter((x) => x.key !== i.key) });
      reload();
      toast(PICK_DONE[pick](p, today), () =>
        api
          .undoReviewPick(p.id)
          .then(() => {
            load();
            reload();
          })
          .catch((e) => toast(`Cannot undo: ${e}`, undefined, true)),
      );
      return true;
    } catch (e) {
      toast(String(e), undefined, true);
      return false;
    } finally {
      picking.current = false;
    }
  };

  const finish = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const r = await api.finishWeekReview(today);
      setReview(r);
      onClose();
      toast(`Weekly review done. The next one is on ${shortDay(addDays(today, api.WEEK_DAYS), today)}.`);
    } catch (e) {
      toast(`The weekly review is not saved: ${e}`, undefined, true);
    } finally {
      setBusy(false);
    }
  };

  const list = goals?.goals ?? [];
  const done = new Map(review.goals.map((g) => [g.id, g.done]));
  return (
    <section className="review" aria-label="Weekly review">
      <div className="eyebrow">
        <span>↻ Weekly review</span>
        {review.last && <span>· last on {shortDay(review.last, today)}</span>}
        <span className="sp" />
        <button className="linkbtn" onClick={onClose} title={review.due ? "Hide it for now. It comes back tomorrow, or with the Weekly review button." : undefined}>
          {review.due ? "Later" : "Close"}
        </button>
      </div>

      <div className="rv-h">1. Are your goals still right?</div>
      {editGoals || !list.length ? (
        <>
          {!list.length && <div className="rv-empty">You have no goals. Add 3 to 5. AIfred then puts the work that moves them first.</div>}
          <GoalEditor />
          {list.length > 0 && (
            <div className="rv-acts">
              <button className="btn" onClick={() => setEditGoals(false)}>
                Done
              </button>
            </div>
          )}
        </>
      ) : (
        <>
          <ul className="rv-list">
            {list.map((g) => {
              const n = done.get(g.id) ?? 0;
              return (
                <li key={g.id} className="rv-row">
                  <span className="rv-title">◎ {g.text}</span>
                  <span className={`rv-sub${n ? "" : " warn"}`}>{n ? `${n} item${n > 1 ? "s" : ""} done this week` : "Nothing done for it this week"}</span>
                </li>
              );
            })}
          </ul>
          <div className="rv-acts">
            <button className="btn" onClick={() => changeGoals(api.confirmGoals).then((ok) => ok && toast("The goals are still right."))}>
              Still right
            </button>
            <button className="btn" onClick={() => setEditGoals(true)}>
              Change them
            </button>
          </div>
        </>
      )}

      <div className="rv-h">
        2. No change for 3 weeks <span className="c">{review.items.length + review.more}</span>
      </div>
      {review.items.length ? (
        <>
          <div className="rv-sub">For each item, pick one. A pick has an Undo.</div>
          <ul className="rv-list">
            {review.items.map((i) => (
              <StaleRow key={i.key} item={i} onPick={onPick} />
            ))}
          </ul>
          {review.more > 0 && <div className="rv-sub">{review.more} more. They come in the next review.</div>}
        </>
      ) : (
        <div className="rv-empty">Nothing is stuck. Each open item changed in the last 3 weeks.</div>
      )}
      <div className="rv-acts">
        <button className="btn pri" disabled={busy} onClick={finish}>
          Finish the review
        </button>
      </div>
    </section>
  );
}
