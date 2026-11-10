// Calendar: your Google Calendar events and your plan, for today or for the
// next 7 days, as in Google Calendar. AIfred plans the day, or you plan it
// yourself. A plan item without a fixed time goes in the next free time,
// around lunch, the meetings, and the items with a fixed time.
//
// Drag a block to a new time: it gets that fixed time. In the week view,
// drag it to another day. Drag an item from "Not in the plan" onto the
// calendar to add it at that time, and drag a block onto that list to take
// it out. Without a mouse: Alt+Up and Alt+Down move a block by 15 minutes,
// and Alt+Left and Alt+Right move it to the day before or after.
//
// Google Calendar does not change: the blocks are only in AIfred.
import { useEffect, useRef, useState } from "react";
import { Composer, ICON, Messages } from "./Chat";
import { useApp } from "./ctx";
import { GoalTag } from "./Goals";
import { DEFAULT_MINUTES, WEEK, addDays, byPlan, dayName, dur, fmtTime, isItemBlock, shortDay, sideBySide, timeOf, workDay, type Item } from "./items";
import type * as api from "./api";
import { Check, DueChip, PriPill, SrcLine } from "./Home";
import { BlockedMark, useOpenBlockers } from "./Links";

/** Pixels for one minute of the calendar: the day view has room for the
 * time, the reason, and the buttons of a 30-minute block. */
const pxOf = (full: boolean) => (full ? 2 : 1);
/** A drag places the block on this step, in minutes. */
const SNAP = 15;
/** The shortest height of a block, in pixels, so its title can show. */
const MIN_H = 22;

/** − and + change the time guess in steps of 15 minutes. */
export function Minutes({ item }: { item: Item }) {
  const { setMinutes } = useApp();
  return (
    <span className="est" title="How long it takes">
      <button aria-label="Less time" onClick={() => setMinutes(item.key, Math.max(5, item.minutes - 15))} disabled={item.minutes <= 5}>
        −
      </button>
      <span>{dur(item.minutes)}</span>
      <button aria-label="More time" onClick={() => setMinutes(item.key, Math.min(600, item.minutes + 15))}>
        +
      </button>
    </span>
  );
}

function Chooser({ day }: { day: string }) {
  const { items, today, plans, startPlan, planChat } = useApp();
  const plan = plans[day] ?? null;
  // Items that a review put in the plan of the day. Both ways keep them.
  const fromReview = plan?.mode === "review" ? plan.items.length : 0;
  const open = items.filter((i) => !i.done && !(i.start && i.start > day));
  const dueN = open.filter((i) => i.due !== null && i.due <= day).length;
  const urgent = open.filter((i) => i.priority === "urgent" || (i.due !== null && i.due < day)).length;
  const name = dayName(day, today);
  return (
    <div className="chooser">
      <h3>How do you want to plan {name}?</h3>
      <p>
        You have {open.length} open items. {dueN} {dueN === 1 ? "is" : "are"} due {day === today ? "today" : `on ${name}`} or late, and {urgent} {urgent === 1 ? "is" : "are"} urgent or late.
      </p>
      {fromReview > 0 && (
        <p>
          Your review put {fromReview} item{fromReview > 1 ? "s" : ""} in this plan. Both ways keep {fromReview > 1 ? "them" : "it"}.
        </p>
      )}
      <div className="choices">
        <button className="choice ai" disabled={planChat.busy} onClick={() => startPlan("ai", day)}>
          <span className="n">{ICON.spark}Plan it for me</span>
          <span className="d">AIfred puts your items in the free time around your meetings, guesses how long each takes, and says why.</span>
          <ul>
            <li>One AI call on your Claude login</li>
            <li>You change it after, by chat or by dragging the blocks</li>
          </ul>
        </button>
        <button className="choice" onClick={() => startPlan("self", day)}>
          <span className="n">{ICON.hand}I plan it myself</span>
          <span className="d">You pick the items and drag them to a time. AIfred fits the rest around lunch and your meetings.</span>
          <ul>
            <li>No AI call until you ask</li>
            <li>Ask AIfred to fill the rest or to check it</li>
          </ul>
        </button>
      </div>
      <p className="peek">
        You can switch at any time. Today's plan shows on the home screen as <b>Next: …</b>
      </p>
    </div>
  );
}

/** The minutes from the first to the last hour that the calendar shows for `days`. */
function hoursOf(days: string[], week: ReturnType<typeof useApp>["week"], s: api.Settings): [number, number] {
  let [lo, hi] = workDay(s);
  for (const d of days) {
    for (const b of week[d]?.sched.all ?? []) {
      lo = Math.min(lo, b.start);
      hi = Math.max(hi, b.end);
    }
    for (const m of week[d]?.events.timed ?? []) {
      lo = Math.min(lo, m.start);
      hi = Math.max(hi, m.end);
    }
  }
  lo = Math.max(0, Math.floor(lo / 60) * 60 - 60);
  hi = Math.min(1440, Math.ceil(hi / 60) * 60 + 60);
  return [lo, hi];
}

/** The drag of the calendar. It works on the page directly, so React does not draw during the drag. */
function useCalDrag(root: React.RefObject<HTMLDivElement | null>, top: number, PX: number) {
  const { addToPlan, setPlanTime, moveToDay, removeFromPlan } = useApp();
  // The newest values, for the handlers that live for the whole screen.
  const live = useRef({ addToPlan, setPlanTime, moveToDay, removeFromPlan, top, PX });
  live.current = { addToPlan, setPlanTime, moveToDay, removeFromPlan, top, PX };

  useEffect(() => {
    const HOLD = 350;
    type Press = { el: HTMLElement; key: string; from: string; min: number; at: number | null; x0: number; y0: number; grab: number; touch: boolean; timer?: number };
    type Drag = Press & { ph: HTMLDivElement; x: number; y: number; to: string | null; toAt: number; out: boolean; tick: number; sc: HTMLElement | null };
    let press: Press | null = null;
    let drag: Drag | null = null;
    let eatClick = false;

    function cancelPress() {
      if (!press) return;
      clearTimeout(press.timer);
      press.el.classList.remove("lift");
      press = null;
    }

    function startDrag(x: number, y: number) {
      const p = press!;
      press = null;
      p.el.classList.remove("lift");
      p.el.classList.add("dragging");
      root.current?.classList.add("sorting");
      const ph = document.createElement("div");
      ph.className = "cal-drop";
      ph.setAttribute("aria-hidden", "true");
      drag = { ...p, ph, x, y, to: null, toAt: 0, out: false, sc: root.current?.closest<HTMLElement>(".scroll") ?? null, tick: 0 };
      drag.tick = window.setInterval(() => {
        if (!drag?.sc) return;
        const r = drag.sc.getBoundingClientRect();
        const edge = 48;
        if (drag.y < r.top + edge) drag.sc.scrollTop -= 9;
        else if (drag.y > r.bottom - edge) drag.sc.scrollTop += 9;
        else return;
        place();
      }, 16);
      place();
    }

    function place() {
      const d = drag!;
      const cols = [...(root.current?.querySelectorAll<HTMLElement>(".cal-col") ?? [])];
      const off = root.current?.querySelector<HTMLElement>(".cal-off");
      const col = cols.find((c) => {
        const r = c.getBoundingClientRect();
        return d.x >= r.left && d.x < r.right && d.y >= r.top && d.y < r.bottom;
      });
      const offR = off?.getBoundingClientRect();
      d.out = !col && !!offR && d.y >= offR.top && d.from !== "";
      if (col) {
        const r = col.getBoundingClientRect();
        const { top, PX } = live.current;
        const m = top + (d.y - d.grab - r.top) / PX;
        d.to = col.dataset.day!;
        d.toAt = Math.max(0, Math.min(1440 - d.min, Math.round(m / SNAP) * SNAP));
        if (d.ph.parentElement !== col) col.appendChild(d.ph);
        d.ph.style.top = `${(d.toAt - top) * PX}px`;
        d.ph.style.height = `${Math.max(MIN_H, d.min * PX)}px`;
        d.ph.textContent = `${fmtTime(d.toAt)}–${fmtTime(d.toAt + d.min)}`;
      } else {
        d.to = null;
        d.ph.remove();
      }
      off?.classList.toggle("drop-out", d.out);
    }

    function endDrag(ok: boolean) {
      cancelPress();
      if (!drag) return;
      const d = drag;
      drag = null;
      clearInterval(d.tick);
      eatClick = true;
      setTimeout(() => (eatClick = false), 0);
      d.ph.remove();
      d.el.classList.remove("dragging");
      root.current?.classList.remove("sorting");
      root.current?.querySelector(".cal-off")?.classList.remove("drop-out");
      if (!ok) return;
      const L = live.current;
      if (d.to) {
        if (d.from === "") L.addToPlan(d.key, { day: d.to, at: d.toAt });
        else if (d.from === d.to) {
          if (d.toAt !== d.at) L.setPlanTime(d.key, d.toAt, d.to);
        } else L.moveToDay(d.key, d.from, d.to, d.toAt);
      } else if (d.out) {
        L.removeFromPlan(d.key, d.from);
      }
    }

    const down = (e: PointerEvent) => {
      if (e.button > 0 || drag) return;
      const t = e.target as HTMLElement;
      if (!root.current?.contains(t) || t.closest("button, input, select, textarea, a")) return;
      const el = t.closest<HTMLElement>("[data-drag]");
      if (!el) return;
      const r = el.getBoundingClientRect();
      const from = el.dataset.day ?? "";
      press = {
        el,
        key: el.dataset.drag!,
        from,
        min: Number(el.dataset.min) || DEFAULT_MINUTES,
        at: el.dataset.at ? Number(el.dataset.at) : null,
        x0: e.clientX,
        y0: e.clientY,
        // A block keeps the place where you hold it. A row from the list hangs from its top.
        grab: from ? e.clientY - r.top : 0,
        touch: e.pointerType !== "mouse",
      };
      if (press.touch) {
        el.classList.add("lift");
        press.timer = window.setTimeout(() => {
          if (!press) return;
          startDrag(press.x0, press.y0);
          try {
            navigator.vibrate?.(12);
          } catch {
            /* no vibration */
          }
        }, HOLD);
      }
    };
    const move = (e: PointerEvent) => {
      if (press) {
        const dist = Math.hypot(e.clientX - press.x0, e.clientY - press.y0);
        if (press.touch) {
          if (dist > 8) cancelPress();
        } else if (dist > 5) startDrag(e.clientX, e.clientY);
      }
      if (drag) {
        drag.x = e.clientX;
        drag.y = e.clientY;
        place();
      }
    };
    const up = () => endDrag(true);
    const cancel = () => endDrag(false);
    const touch = (e: TouchEvent) => drag && e.preventDefault();
    const menu = (e: Event) => (drag || press) && e.preventDefault();
    const click = (e: MouseEvent) => {
      if (eatClick) {
        e.stopPropagation();
        e.preventDefault();
        eatClick = false;
      }
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && (drag || press)) {
        e.stopImmediatePropagation();
        endDrag(false);
      }
    };
    document.addEventListener("pointerdown", down);
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", cancel);
    document.addEventListener("touchmove", touch, { passive: false });
    document.addEventListener("contextmenu", menu);
    document.addEventListener("click", click, true);
    document.addEventListener("keydown", key, true);
    return () => {
      endDrag(false);
      document.removeEventListener("pointerdown", down);
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.removeEventListener("pointercancel", cancel);
      document.removeEventListener("touchmove", touch);
      document.removeEventListener("contextmenu", menu);
      document.removeEventListener("click", click, true);
      document.removeEventListener("keydown", key, true);
    };
  }, [root]);
}

/** A calendar event as a block. */
function EventBlock({ m, full }: { m: { start: number; end: number; event: api.CalEvent }; full: boolean }) {
  const { planChat, sendPlan, calDay, today } = useApp();
  const e = m.event;
  const time = `${fmtTime(m.start)}–${fmtTime(m.end)}`;
  const who = e.attendees.slice(0, 3).join(", ") + (e.attendees.length > 3 || e.more_attendees ? ` +${e.attendees.length - 3 + e.more_attendees}` : "");
  return (
    <>
      <span className="cb-title">{e.url ? <a href={e.url}>{e.title}</a> : e.title}</span>
      <span className="cb-meta">
        <span>{time}</span>
        {full && who && <span>{who}</span>}
        {!e.busy && <span>Free</span>}
        {e.response && e.response !== "accepted" && <span className="pill">{e.response === "tentative" ? "Maybe" : "Not answered"}</span>}
      </span>
      {full && (
        <span className="ctl">
          <button
            className="x"
            title="AIfred writes a short note: who attends, what it is for, and your linked items"
            aria-label={`Prepare for: ${e.title}`}
            disabled={planChat.busy}
            onClick={() =>
              // The title is not in the message: the invite sender wrote it, and
              // this message goes to the AI as the user's own words.
              sendPlan(`Prepare me for my calendar event ${calDay === today ? "today" : `on ${calDay}`} at ${time} (event id ${e.id}): who attends, what it is for, and which of my open items and goals link to it.`)
            }
          >
            Prep
          </button>
        </span>
      )}
    </>
  );
}

/** A plan item as a block. */
function ItemBlock({ i, day, start, end, pinned, full }: { i: Item; day: string; start: number; end: number; pinned: boolean; full: boolean }) {
  const { plans, open, removeFromPlan, setPlanTime, moveToDay, goalText, brief, today } = useApp();
  const p = plans[day]?.items.find((x) => x.key === i.key);
  const mission = brief?.missions?.find((m) => m.status === "open" && m.items.some((x) => x.key === i.key));
  const last = addDays(today, WEEK - 1);
  const keys = (e: React.KeyboardEvent) => {
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      // Alt+Up and Alt+Down move the block by 15 minutes, without a mouse.
      e.preventDefault();
      const to = Math.max(0, Math.min(1440 - i.minutes, start + (e.key === "ArrowUp" ? -SNAP : SNAP)));
      setPlanTime(i.key, to, day);
      return;
    }
    if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      // Alt+Left and Alt+Right move the block to the day before or after.
      e.preventDefault();
      const to = addDays(day, e.key === "ArrowLeft" ? -1 : 1);
      if (to >= today && to <= last) moveToDay(i.key, day, to, pinned ? start : null);
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open(i.key);
    }
  };
  const why = p?.why ?? (day === today ? i.why : undefined);
  const blockedBy = useOpenBlockers(i.key);
  // The goal that AIfred gave in the plan, else the goal from the brief.
  const goal = goalText(p?.goal) ?? (p?.why ? undefined : i.goal);
  return (
    <>
      {full && <Check item={i} />}
      <div className="t-main" role="button" tabIndex={0} aria-label={`Open details: ${i.title}`} title={full ? undefined : i.title} onClick={() => open(i.key)} onKeyDown={keys}>
        <span className="cb-title">{i.title}</span>
        <span className="cb-meta">
          <span>
            {fmtTime(start)}–{fmtTime(end)}
          </span>
          {full && <span className={`src ${i.src}`}>{i.ref || "Mine"}</span>}
          {full && <PriPill p={i.priority} />}
          {full && mission && mission.title !== i.title && <span className="cb-mission">Mission: {mission.title}</span>}
          {!i.done && blockedBy.length > 0 && <span className="cb-blocked">Blocked by {blockedBy.map((b) => b.title).join(", ")}</span>}
        </span>
        {full && why && !i.done && (
          <span className="why">
            {why} <GoalTag goal={goal} ranked />
          </span>
        )}
      </div>
      {full && (
        <span className="ctl">
          {pinned && (
            <button className="pin" title={`Fixed at ${fmtTime(start)}. Click to let AIfred put it in the next free time.`} aria-label={`No fixed time: ${i.title}`} onClick={() => setPlanTime(i.key, null, day)}>
              {fmtTime(start)} ✕
            </button>
          )}
          <Minutes item={i} />
          <button title="Take out of the plan" aria-label={`Take out of the plan: ${i.title}`} onClick={() => removeFromPlan(i.key, day)}>
            ✕
          </button>
        </span>
      )}
    </>
  );
}

/** One day column: the hour lines, the events, lunch, and the plan items. */
/** `firstDay`: the first day on screen of each planned item. One item can be
 * in the plans of two days, but only one block can have its id: the open
 * panel scrolls to the block with that id. */
function DayColumn({ day, top, bottom, full, firstDay }: { day: string; top: number; bottom: number; full: boolean; firstDay: Map<string, string> }) {
  const { week, settings, sel, today } = useApp();
  const PX = pxOf(full);
  const sched = week[day]?.sched;
  const items = week[day]?.items ?? [];
  const byKey = new Map(items.map((i) => [i.key, i]));
  type B = { key: string; start: number; end: number; node: React.ReactNode; cls: string; drag?: { key: string; min: number; at: number | null } };
  const blocks: B[] = [];
  for (const m of week[day]?.events.timed ?? []) {
    blocks.push({ key: `event:${m.event.id}`, start: m.start, end: m.end, cls: `cb meet${m.event.busy ? "" : " free"}`, node: <EventBlock m={m} full={full} /> });
  }
  const ls = timeOf(settings.lunch_start);
  if (settings.lunch_minutes > 0 && ls !== null) {
    blocks.push({
      key: "lunch",
      start: ls,
      end: ls + settings.lunch_minutes,
      cls: "cb lunch",
      node: (
        <>
          <span className="cb-title">Lunch</span>
          {full && <span className="cb-meta">From your settings: {settings.lunch_minutes} minutes</span>}
        </>
      ),
    });
  }
  for (const b of sched?.all ?? []) {
    if (!isItemBlock(b)) continue;
    const i = byKey.get(b.key);
    if (!i) continue;
    const pinned = "pinned" in b && !!b.pinned;
    blocks.push({
      key: i.key,
      start: b.start,
      end: b.end,
      cls: `cb item s-${i.src}${i.done ? " done" : ""}${sel === i.key ? " sel" : ""}${pinned ? " pinned" : ""}`,
      drag: { key: i.key, min: i.minutes, at: pinned ? b.start : null },
      node: <ItemBlock i={i} day={day} start={b.start} end={b.end} pinned={pinned} full={full} />,
    });
  }
  const place = sideBySide(blocks);
  const [ws, we] = workDay(settings);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (day !== today) return;
    const t = setInterval(() => setNow(new Date()), 60_000);
    return () => clearInterval(t);
  }, [day, today]);
  const nowMin = now.getHours() * 60 + now.getMinutes();
  return (
    <div className="cal-col" data-day={day} role="group" aria-label={`Calendar of ${shortDay(day, today)}`} style={{ height: (bottom - top) * PX, ["--hour" as string]: `${60 * PX}px` }}>
      {/* The time outside the work day is grey. */}
      <div className="cal-off-hours" style={{ top: 0, height: Math.max(0, (ws - top) * PX) }} />
      <div className="cal-off-hours" style={{ top: (we - top) * PX, height: Math.max(0, (bottom - we) * PX) }} />
      {blocks.map((b, n) => {
        const { col, cols } = place[n];
        const isItem = !!b.drag;
        return (
          <div
            key={b.key}
            className={b.cls}
            id={isItem && firstDay.get(b.key) === day ? `row-${b.key}` : undefined}
            data-drag={b.drag?.key}
            data-day={b.drag ? day : undefined}
            data-min={b.drag?.min}
            data-at={b.drag?.at ?? undefined}
            style={{
              top: (b.start - top) * PX,
              height: Math.max(MIN_H, (b.end - b.start) * PX - 2),
              left: `calc(${(col / cols) * 100}% + 2px)`,
              width: `calc(${100 / cols}% - 4px)`,
            }}
          >
            {b.node}
          </div>
        );
      })}
      {day === today && nowMin >= top && nowMin <= bottom && <div className="cal-now" style={{ top: (nowMin - top) * PX }} aria-hidden="true" />}
    </div>
  );
}

/** "1 h 30 m too much", or "2 h free". */
function FreeText({ free }: { free: number }) {
  return free < 0 ? <span className="over">Too full by {dur(-free)}</span> : <span>{dur(free)} free</span>;
}

function Grid({ days }: { days: string[] }) {
  const { week, settings, today, calView, pickDay, setCalView, plans, items, sel, open, addToPlan, calDay } = useApp();
  const full = calView === "day";
  const [top, bottom] = hoursOf(days, week, settings);
  const PX = pxOf(full);
  const root = useRef<HTMLDivElement>(null);
  useCalDrag(root, top, PX);
  const hours: number[] = [];
  for (let h = top; h <= bottom; h += 60) hours.push(h);
  // In the day view: the items that are not in the plan of the day. In the
  // week view: the items that are in no plan of the week.
  const planned = new Set(days.flatMap((d) => (plans[d]?.items ?? []).map((p) => p.key)));
  const firstDay = new Map<string, string>();
  for (const d of days) for (const i of week[d]?.items ?? []) if (!firstDay.has(i.key)) firstDay.set(i.key, d);
  // A task that starts later can go only on its start day or after it. In
  // the week view it shows when it can go on one of the days.
  const lastDay = full ? calDay : days[days.length - 1];
  const outside = items.filter((i) => !i.done && !planned.has(i.key) && !(i.start && i.start > lastDay)).sort(byPlan);
  /** The day that "Add" puts the item on: the day on screen (today in the
   * week view), or the start day of a task that starts later. */
  const base = full ? calDay : today;
  const addDay = (i: Item) => (i.start && i.start > base ? i.start : base);
  const offName = full ? (calDay === today ? "Not in today's plan" : `Not in the plan for ${shortDay(calDay, today)}`) : "Not planned this week";
  const allDay = days.map((d) => week[d]?.events.allDay ?? []);
  // The drop place to take a block out of the plan shows also when every item is planned.
  const anyPlanned = days.some((d) => (week[d]?.items.length ?? 0) > 0);
  return (
    <div className="cal-wrap" ref={root}>
      <div className={`cal ${calView}`} style={{ ["--cols" as string]: days.length }}>
        {!full && (
          <div className="cal-head">
            <span />
            {days.map((d) => {
              const s = week[d]?.sched;
              const has = (week[d]?.items.length ?? 0) > 0;
              return (
                <button key={d} className={`cal-dh${d === today ? " today" : ""}`} onClick={() => (pickDay(d), setCalView("day"))} aria-label={`Open ${shortDay(d, today)}`}>
                  <b>{d === today ? "Today" : shortDay(d, today)}</b>
                  <span className="cal-dh-s">{has && s ? <FreeText free={s.free} /> : plans[d] ? "Empty plan" : "No plan"}</span>
                </button>
              );
            })}
          </div>
        )}
        {allDay.some((a) => a.length) && (
          <div className="cal-allday">
            <span className="cal-hl">All day</span>
            {allDay.map((list, n) => (
              <div key={days[n]}>
                {list.map((e) => (
                  <span key={e.id} className="cal-ad" title={e.title}>
                    {e.url ? <a href={e.url}>{e.title}</a> : e.title}
                  </span>
                ))}
              </div>
            ))}
          </div>
        )}
        <div className="cal-body">
          <div className="cal-hours" style={{ height: (bottom - top) * PX }}>
            {hours.map((h) => (
              <span key={h} style={{ top: (h - top) * PX }}>
                {fmtTime(h)}
              </span>
            ))}
          </div>
          {days.map((d) => (
            <DayColumn key={d} day={d} top={top} bottom={bottom} full={full} firstDay={firstDay} />
          ))}
        </div>
      </div>
      {(outside.length > 0 || anyPlanned) && (
        <div className="off cal-off">
          <div className="group-h">
            {offName} <span className="c">{outside.length}</span>
            <span className="cal-hint">Drag an item onto the calendar, or drag a block here to take it out.</span>
          </div>
          {!outside.length && <p className="cal-hint">Every open item is in the plan.</p>}
          <ul className="list" aria-label={offName}>
            {outside.map((i) => (
              <li key={i.key} className={`row${sel === i.key ? " sel" : ""}`} data-drag={i.key} data-min={i.minutes || DEFAULT_MINUTES}>
                <div
                  className="t-main"
                  role="button"
                  tabIndex={0}
                  aria-label={`Open details: ${i.title}`}
                  onClick={() => open(i.key)}
                  onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), open(i.key))}
                >
                  <span className="t-title">{i.title}</span>
                  <SrcLine item={i} extra={<><DueChip day={i.due} /><PriPill p={i.priority} /></>} />
                </div>
                <BlockedMark item={i} />
                <button className="btn" onClick={() => addToPlan(i.key, { day: addDay(i) })} title={`Add to ${dayName(addDay(i), today)}`}>
                  Add · {dur(i.minutes || DEFAULT_MINUTES)}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

export function CalendarView(props: { onChat: () => void }) {
  const { plans, week, today, calDay, pickDay, calView, setCalView, sendPlan, startPlan, startOver, planChat } = useApp();
  const days = Array.from({ length: WEEK }, (_, n) => addDays(today, n));
  const plan = plans[calDay] ?? null;
  const { items: dayItems, sched } = week[calDay] ?? { items: [], sched: null };
  const making = planChat.busy && !dayItems.length;
  const noPlan = !plan || plan.mode === "review";
  const first = calDay <= today;
  const last = calDay >= days[days.length - 1];
  const name = dayName(calDay, today);
  return (
    <div className="c-mid">
      <div className="c-top">
        <h2>Calendar</h2>
        <div className="seg" role="group" aria-label="View">
          <button aria-pressed={calView === "day"} onClick={() => setCalView("day")}>
            Day
          </button>
          <button aria-pressed={calView === "week"} onClick={() => setCalView("week")}>
            Week
          </button>
        </div>
        {calView === "day" && (
          <span className="cal-nav">
            <button className="linkbtn" aria-label="The day before" disabled={first} onClick={() => pickDay(addDays(calDay, -1))}>
              ‹
            </button>
            <b>{calDay === today ? `Today, ${shortDay(today, today)}` : shortDay(calDay, today)}</b>
            <button className="linkbtn" aria-label="The next day" disabled={last} onClick={() => pickDay(addDays(calDay, 1))}>
              ›
            </button>
            {calDay !== today && (
              <button className="linkbtn" onClick={() => pickDay(today)}>
                Today
              </button>
            )}
          </span>
        )}
        {calView === "day" && !noPlan && <span className="mode">{plan.mode === "ai" ? "✦ Made by AIfred" : "Made by you"}</span>}
        {calView === "day" && !noPlan && dayItems.length > 0 && sched && (
          <div className="c-stats">
            <span>
              Work <b>{dur(sched.work)}</b>
            </span>
            <span>
              Ends <b>{fmtTime(sched.end ?? 0)}</b>
            </span>
            <span>
              {sched.free < 0 ? (
                <>
                  Too much <b className="over">{dur(-sched.free)}</b>
                </>
              ) : (
                <>
                  Free <b>{dur(sched.free)}</b>
                </>
              )}
            </span>
          </div>
        )}
        <span className="right">
          <button className="btn m-only" onClick={props.onChat}>
            Plan chat
          </button>
          {calView === "day" && !noPlan && (
            <>
              {plan.mode === "self" ? (
                <button className="btn pri" disabled={planChat.busy} onClick={() => sendPlan("Fill the rest of my day plan. Keep my items, their order, and their times, and add items after them.")}>
                  Let AIfred fill the rest
                </button>
              ) : (
                <button className="btn" disabled={planChat.busy} onClick={() => startPlan("ai", calDay)}>
                  Make it again
                </button>
              )}
              <button className="linkbtn" disabled={planChat.busy} onClick={() => startOver(calDay)}>
                Start over
              </button>
            </>
          )}
        </span>
      </div>
      <div className="scroll">
        {calView === "day" && noPlan && !making && <Chooser day={calDay} />}
        {calView === "day" && making && (
          <div className="tl-empty cal-note">
            <strong>AIfred is making your plan…</strong>
            <span>It reads your open items, the due days, your meetings, and your memory.</span>
          </div>
        )}
        {calView === "day" && !noPlan && !making && !dayItems.length && (
          <div className="tl-empty cal-note">
            <strong>Your plan is empty</strong>
            <span>Drag items from the list below onto the calendar, or ask AIfred to fill it.</span>
          </div>
        )}
        {calView === "day" && !noPlan && sched && sched.free < 0 && dayItems.length > 0 && (
          <div className="cal-warn" role="status">
            <span>
              This day is too full by <b>{dur(-sched.free)}</b>. Move some items to another day, or ask AIfred what can wait.
            </span>
            <button className="btn" disabled={planChat.busy} onClick={() => sendPlan(`My plan for ${name} is too full. Which items can move to another day?`)}>
              What can wait?
            </button>
          </div>
        )}
        {calView === "week" && days.some((d) => (week[d]?.items.length ?? 0) > 0 && (week[d]?.sched.free ?? 0) < 0) && (
          <div className="cal-warn" role="status">
            <span>
              Too full:{" "}
              {days
                .filter((d) => (week[d]?.items.length ?? 0) > 0 && (week[d]?.sched.free ?? 0) < 0)
                .map((d) => `${dayName(d, today)} by ${dur(-(week[d]?.sched.free ?? 0))}`)
                .join(", ")}
              . Drag some blocks to a day with free time.
            </span>
          </div>
        )}
        <Grid days={calView === "day" ? [calDay] : days} />
      </div>
    </div>
  );
}

/** The plan chat, on the right side of the Calendar page. It belongs to the day on the page. */
export function PlanChat() {
  const { plans, calDay, today, planChat, sendPlan, models, setModel, open, byKey } = useApp();
  const plan = plans[calDay] ?? null;
  const msgs = useRef<HTMLDivElement>(null);
  useEffect(() => {
    msgs.current?.scrollTo({ top: msgs.current.scrollHeight });
  }, [planChat.msgs.length, planChat.busy, planChat.progress.length]);
  const name = dayName(calDay, today);
  const intro =
    !plan || plan.mode === "review"
      ? `Pick one of the two ways on the left to plan ${name}. You can also type here, for example “plan my day, but keep the afternoon free”.`
      : plan.mode === "self" && !planChat.msgs.length
        ? "Drag items from the list under the calendar to a time, or press Add. I place the items without a fixed time in the free time around lunch and your meetings. Change the time with − and +.\n\nWhen you want help, ask me to fill the rest or to check the plan."
        : null;
  return (
    <>
      <div className="msgs" ref={msgs}>
        <div className="chat-day">Plan chat for {name}</div>
        {intro && <Messages list={[{ role: "intro", text: intro }]} />}
        <Messages list={planChat.msgs} busy={planChat.busy} progress={planChat.progress} onItem={open} hasItem={(k) => byKey.has(k)} busyText="AIfred is looking at your day…" />
      </div>
      <div className="chat-in">
        <Composer
          id="plan"
          placeholder={plan ? "Change the plan…" : "Plan my day…"}
          busy={planChat.busy}
          model={models.plan}
          onModel={(m) => setModel("plan", m)}
          suggestions={plan ? ["Start with the most urgent item", "Is this too much for one day?", "Move the meeting prep to 14:00", "Fill the rest for me"] : ["Plan my day", "Plan my day, keep the afternoon free"]}
          onSend={(text, files) => sendPlan(text, files)}
        />
      </div>
    </>
  );
}
