// Day plan: AIfred plans the day, or you plan it yourself. The items go on
// the work day in your order, around lunch. Press and hold an item to drag
// it: the other items slide apart where it will go. Drop an item of the plan
// on "Not in today's plan" to take it out.
import { useEffect, useRef } from "react";
import { Composer, ICON, Messages } from "./Chat";
import { useApp } from "./ctx";
import { GoalTag } from "./Goals";
import { DEFAULT_MINUTES, byPlan, dur, fmtTime, shortDay, type Item } from "./items";
import { Check, DueChip, PriPill, SrcLine } from "./Home";

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

function Chooser() {
  const { items, today, startPlan, planChat } = useApp();
  const open = items.filter((i) => !i.done && !(i.start && i.start > today));
  const todayN = open.filter((i) => i.due !== null && i.due <= today).length;
  const urgent = open.filter((i) => i.priority === "urgent" || (i.due !== null && i.due < today)).length;
  return (
    <div className="scroll">
      <div className="chooser">
        <h3>How do you want to plan today?</h3>
        <p>
          You have {open.length} open items. {todayN} {todayN === 1 ? "is" : "are"} due today or late, and {urgent} {urgent === 1 ? "is" : "are"} urgent or late.
        </p>
        <div className="choices">
          <button className="choice ai" disabled={planChat.busy} onClick={() => startPlan("ai")}>
            <span className="n">{ICON.spark}Plan it for me</span>
            <span className="d">AIfred puts your items in order, guesses how long each takes, and says why.</span>
            <ul>
              <li>One AI call on your Claude login</li>
              <li>You change it after, by chat or by hand</li>
            </ul>
          </button>
          <button className="choice" onClick={() => startPlan("self")}>
            <span className="n">{ICON.hand}I plan it myself</span>
            <span className="d">You pick the items and the order. AIfred fits them around lunch.</span>
            <ul>
              <li>No AI call until you ask</li>
              <li>Ask AIfred to fill the rest or to check it</li>
            </ul>
          </button>
        </div>
        <p className="peek">
          You can switch at any time. Your plan shows on the home screen as <b>Next: …</b>
        </p>
      </div>
    </div>
  );
}

type DragItem = { el: HTMLElement; k: number; top: number; h: number; plan: boolean; mv: boolean; off: boolean };

/** The drag of the plan. It works on the page directly, so React does not draw during the drag. */
function useDrag(tl: React.RefObject<HTMLDivElement | null>) {
  const { planItems, savePlan, addToPlan, removeFromPlan, byKey } = useApp();
  // The newest values, for the handlers that live for the whole screen.
  const live = useRef({ planItems, savePlan, addToPlan, removeFromPlan, byKey });
  live.current = { planItems, savePlan, addToPlan, removeFromPlan, byKey };

  useEffect(() => {
    const HOLD = 350;
    type Press = { el: HTMLElement; id: string; from: "plan" | "off"; x0: number; y0: number; y: number; touch: boolean; timer?: number };
    type Drag = {
      id: string; from: "plan" | "off"; el: HTMLElement; tl: HTMLElement; sc: HTMLElement | null; off: HTMLElement | null;
      items: DragItem[]; me: DragItem; ph: HTMLDivElement; H: number; others: DragItem[]; idx: number; to: number;
      zone: "plan" | "out"; y0: number; s0: number; y: number; empty: HTMLElement | null; tick: number;
    };
    let press: Press | null = null;
    let drag: Drag | null = null;
    let eatClick = false;

    function cancelPress() {
      if (!press) return;
      clearTimeout(press.timer);
      press.el.classList.remove("lift");
      press = null;
    }

    function startDrag() {
      const p = press!;
      press = null;
      const root = p.el.closest(".tl") as HTMLElement;
      const sc = root.closest(".scroll") as HTMLElement | null;
      const off = root.querySelector(".off") as HTMLElement | null;
      const base = root.getBoundingClientRect().top;
      // Everything that can slide, in screen order. Positions are read once, so the drop place does not jump.
      const els = [...root.querySelectorAll<HTMLElement>(":scope > .slot")].concat(off ? [...off.querySelectorAll<HTMLElement>(".group-h, .row")] : []);
      const items: DragItem[] = els.map((el, k) => {
        const r = el.getBoundingClientRect();
        return { el, k, top: r.top - base, h: r.height, plan: el.classList.contains("slot"), mv: !!el.dataset.mv, off: !!el.dataset.off };
      });
      const me = items.find((it) => it.el === p.el)!;
      const ph = document.createElement("div");
      ph.className = "ph";
      root.appendChild(ph);
      const others = items.filter((it) => it.mv && it !== me);
      drag = {
        id: p.id, from: p.from, el: p.el, tl: root, sc, off, items, me, ph, H: me.h + (me.plan ? 6 : 8), others,
        idx: others.filter((it) => it.k < me.k).length, to: -1, zone: "plan", y0: p.y0, s0: sc?.scrollTop ?? 0, y: p.y,
        empty: root.querySelector(".tl-empty"), tick: 0,
      };
      p.el.classList.add("dragging");
      root.classList.add("sorting");
      drag.tick = window.setInterval(() => {
        if (!drag?.sc) return;
        const r = drag.sc.getBoundingClientRect();
        const edge = 56;
        if (drag.y < r.top + edge) drag.sc.scrollTop -= 9;
        else if (drag.y > r.bottom - edge) drag.sc.scrollTop += 9;
        else return;
        place();
      }, 16);
      place();
    }

    function place() {
      const d = drag!;
      const me = d.me;
      d.el.style.transform = `translateY(${d.y - d.y0 + (d.sc ? d.sc.scrollTop - d.s0 : 0)}px)`;
      const y = d.y - d.tl.getBoundingClientRect().top;
      const mid = (it: DragItem) => it.top + it.h / 2;
      const plans = d.items.filter((it) => it.plan && it !== me);
      const offTop = d.off ? d.off.getBoundingClientRect().top - d.tl.getBoundingClientRect().top - 8 : Infinity;
      d.zone = d.off && y >= offTop ? "out" : "plan";
      let G: number;
      if (d.zone === "plan") {
        const n = d.others.length;
        const to = d.others.filter((it) => mid(it) < y).length;
        d.to = to;
        // Lunch or free time can be between two items. The space goes on the side of them that the pointer is on.
        const prevK = to > 0 ? d.others[to - 1].k : -1;
        const nextK = to < n ? d.others[to].k : plans.length ? plans[plans.length - 1].k + 1 : 0;
        const fixed = plans.filter((it) => !it.mv && it.k > prevK && it.k < nextK);
        const m = fixed.filter((it) => mid(it) < y).length;
        G = m === 0 ? prevK + 0.5 : fixed[m - 1].k + 0.5;
        if (d.from === "plan" && to === d.idx) G = me.k + 0.5;
      } else {
        d.to = -1;
        const firstRow = d.items.find((it) => it.off && it !== me);
        G = d.from === "off" ? me.k + 0.5 : firstRow ? firstRow.k - 0.25 : d.items.length;
      }
      const H = d.H;
      let phTop: number | null = null;
      for (const it of d.items) {
        if (it === me) continue;
        const shift = (it.k > G ? H : 0) - (it.k > me.k ? H : 0);
        it.el.style.transform = shift ? `translateY(${shift}px)` : "";
        if (phTop === null && it.k > G) phTop = it.top + shift - H;
      }
      if (phTop === null) {
        const last = d.items.filter((it) => it !== me).pop();
        phTop = last ? last.top + last.h + (last.k > me.k ? -H : 0) + 6 : 0;
      }
      const still = d.from === "plan" ? d.zone === "plan" && d.to === d.idx : d.zone === "out";
      d.ph.style.top = `${phTop}px`;
      d.ph.style.height = `${H - (me.plan ? 6 : 8)}px`;
      d.ph.className = `ph${d.zone === "out" ? " out" : ""}`;
      d.ph.textContent = d.zone === "out" && d.from === "plan" ? "Let go to take it out of today's plan" : "";
      d.ph.style.opacity = still && d.from === "off" ? "0" : "1";
      d.empty?.classList.toggle("drop-in", d.zone === "plan" && d.from === "off");
    }

    function endDrag(ok: boolean) {
      cancelPress();
      if (!drag) return;
      const d = drag;
      drag = null;
      clearInterval(d.tick);
      eatClick = true;
      setTimeout(() => (eatClick = false), 0);
      // Put the page back as it was. React then draws the new order.
      d.items.forEach((it) => (it.el.style.transform = ""));
      d.el.style.transform = "";
      d.ph.remove();
      d.el.classList.remove("dragging");
      d.tl.classList.remove("sorting");
      d.empty?.classList.remove("drop-in");
      const L = live.current;
      const title = L.byKey.get(d.id)?.title ?? "";
      if (ok && d.zone === "plan" && (d.from === "off" || d.to !== d.idx)) {
        if (d.from === "off") {
          L.addToPlan(d.id, d.to);
        } else {
          const rest = L.planItems.map((i) => i.key).filter((k) => k !== d.id);
          rest.splice(d.to, 0, d.id);
          L.savePlan(rest, `Moved: ${title}`);
        }
      } else if (ok && d.zone === "out" && d.from === "plan") {
        L.removeFromPlan(d.id);
      }
    }

    const down = (e: PointerEvent) => {
      if (e.button > 0 || drag) return;
      const t = e.target as HTMLElement;
      if (!tl.current?.contains(t) || t.closest("button, input, select, textarea, a")) return;
      const el = t.closest<HTMLElement>(".slot[data-mv], .off .row[data-off]");
      if (!el) return;
      press = { el, id: (el.dataset.mv || el.dataset.off)!, from: el.dataset.mv ? "plan" : "off", x0: e.clientX, y0: e.clientY, y: e.clientY, touch: e.pointerType !== "mouse" };
      if (press.touch) {
        el.classList.add("lift");
        press.timer = window.setTimeout(() => {
          if (!press) return;
          press.el.classList.remove("lift");
          startDrag();
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
        press.y = e.clientY;
        if (press.touch) {
          if (dist > 8) cancelPress();
        } else if (dist > 5) startDrag();
      }
      if (drag) {
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
  }, [tl]);
}

function Timeline() {
  const { plan, planItems, sched, byKey, items, today, sel, open, removeFromPlan, addToPlan, planChat, sendPlan, settings, savePlan, goalText } = useApp();
  const tl = useRef<HTMLDivElement>(null);
  useDrag(tl);
  const making = planChat.busy && !planItems.length;
  const outside = items.filter((i) => !i.done && !planItems.includes(i) && !(i.start && i.start > today)).sort(byPlan);
  const keyOpen = (key: string) => (e: React.KeyboardEvent) => {
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      // Alt+Up and Alt+Down move the item without a mouse.
      e.preventDefault();
      const keys = planItems.map((i) => i.key);
      const a = keys.indexOf(key);
      const b = e.key === "ArrowUp" ? a - 1 : a + 1;
      if (a >= 0 && b >= 0 && b < keys.length) {
        [keys[a], keys[b]] = [keys[b], keys[a]];
        savePlan(keys);
      }
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open(key);
    }
  };
  const blocks: React.ReactNode[] = [];
  let prevEnd = sched.all[0]?.start ?? 0;
  for (const b of sched.all) {
    if (b.start - prevEnd >= 15) {
      blocks.push(
        <div className="slot" key={`free-${b.start}`}>
          <span />
          <div className="free">{dur(b.start - prevEnd)} free</div>
        </div>,
      );
    }
    const h = Math.max(46, Math.round((b.end - b.start) * 0.55));
    if (b.lunch) {
      blocks.push(
        <div className="slot lunch" key="lunch">
          <span className="tm">{fmtTime(b.start)}</span>
          <div className="blk fixed" style={{ minHeight: h }}>
            <span />
            <div className="t-main" style={{ cursor: "default" }}>
              <span className="t-title">Lunch</span>
              <div className="meta">From your settings: {settings.lunch_minutes} minutes</div>
            </div>
            <span />
          </div>
        </div>,
      );
    } else if (b.meeting) {
      const m = b.meeting;
      const who = m.attendees.slice(0, 3).join(", ") + (m.attendees.length > 3 || m.more_attendees ? ` +${m.attendees.length - 3 + m.more_attendees}` : "");
      const time = `${fmtTime(b.start)}–${fmtTime(b.end)}`;
      blocks.push(
        <div className="slot meeting" key={b.key}>
          <span className="tm">{fmtTime(b.start)}</span>
          <div className="blk fixed meet" style={{ minHeight: h }}>
            <span />
            <div className="t-main" style={{ cursor: "default" }}>
              <span className="t-title">
                {m.url ? <a href={m.url}>{m.title}</a> : m.title}
              </span>
              <div className="meta">
                <span className="src cal">Calendar</span>
                <span>{time}</span>
                {who && <span>{who}</span>}
                {m.response && m.response !== "accepted" && <span className="pill">{m.response === "tentative" ? "Maybe" : "Not answered"}</span>}
              </div>
            </div>
            <span className="ctl">
              <button
                className="x"
                title="AIfred writes a short note: who attends, what it is for, and your linked items"
                aria-label={`Prepare for: ${m.title}`}
                disabled={planChat.busy}
                onClick={() =>
                  // The title is not in the message: the invite sender wrote it, and
                  // this message goes to the AI as the user's own words.
                  sendPlan(`Prepare me for my calendar event today at ${time} (event id ${m.id}): who attends, what it is for, and which of my open items and goals link to it.`)
                }
              >
                Prep
              </button>
            </span>
          </div>
        </div>,
      );
    } else {
      const i = byKey.get(b.key)!;
      blocks.push(
        <div className="slot" data-mv={i.key} key={i.key}>
          <span className="tm">{fmtTime(b.start)}</span>
          <div className={`blk s-${i.src}${i.done ? " done" : ""}${sel === i.key ? " sel" : ""}`} id={`row-${i.key}`} style={{ minHeight: h }}>
            <Check item={i} />
            <div className="t-main" role="button" tabIndex={0} aria-label={`Open details: ${i.title}`} onClick={() => open(i.key)} onKeyDown={keyOpen(i.key)}>
              <span className="t-title">{i.title}</span>
              <div className="meta">
                <span className={`src ${i.src}`}>{i.ref || "Mine"}</span>
                <span>
                  {fmtTime(b.start)}–{fmtTime(b.end)}
                </span>
                <PriPill p={i.priority} />
              </div>
              {(() => {
                const p = plan?.items.find((p) => p.key === i.key);
                const why = p?.why ?? i.why;
                if (!why || i.done) return null;
                // The goal that AIfred gave in the plan, else the goal from the brief.
                const goal = goalText(p?.goal) ?? (p?.why ? undefined : i.goal);
                return (
                  <span className="why">
                    {why} <GoalTag goal={goal} ranked />
                  </span>
                );
              })()}
            </div>
            <span className="ctl">
              <Minutes item={i} />
              <button title="Take out of the plan" aria-label={`Take out of the plan: ${i.title}`} onClick={() => removeFromPlan(i.key)}>
                ✕
              </button>
            </span>
          </div>
        </div>,
      );
    }
    prevEnd = Math.max(prevEnd, b.end);
  }
  return (
    <div className="scroll">
      <div className="tl" ref={tl}>
        {making ? (
          <div className="tl-empty">
            <strong>AIfred is making your plan…</strong>
            <span>It reads your open items, the due days, and your memory.</span>
          </div>
        ) : !planItems.length ? (
          <div className="tl-empty">
            <strong>Your plan is empty</strong>
            <span>Add items from the list below, or ask AIfred to fill it.</span>
          </div>
        ) : (
          blocks
        )}
        {outside.length > 0 && !making && (
          <div className="off">
            <div className="group-h">
              Not in today's plan <span className="c">{outside.length}</span>
            </div>
            <ul className="list" aria-label="Not in today's plan">
              {outside.map((i) => (
                <li key={i.key} className={`row${sel === i.key ? " sel" : ""}`} data-off={i.key}>
                  <div className="t-main" role="button" tabIndex={0} aria-label={`Open details: ${i.title}`} onClick={() => open(i.key)} onKeyDown={keyOpen(i.key)}>
                    <span className="t-title">{i.title}</span>
                    <SrcLine item={i} extra={<><DueChip day={i.due} /><PriPill p={i.priority} /></>} />
                  </div>
                  <button className="btn" onClick={() => addToPlan(i.key)}>
                    Add · {dur(i.minutes || DEFAULT_MINUTES)}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}

export function PlanView(props: { onChat: () => void }) {
  const { plan, planItems, sched, today, sendPlan, startPlan, startOver, planChat } = useApp();
  if (!plan) {
    return (
      <div className="c-mid only">
        <Chooser />
      </div>
    );
  }
  return (
    <div className="c-mid">
      <div className="c-top">
        <h2>Today, {shortDay(today, today)}</h2>
        <span className="mode">{plan.mode === "ai" ? "✦ Made by AIfred" : "Made by you"}</span>
        {planItems.length > 0 && (
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
          {plan.mode === "self" ? (
            <button className="btn pri" disabled={planChat.busy} onClick={() => sendPlan("Fill the rest of my day plan. Keep my items and their order, and add items after them.")}>
              Let AIfred fill the rest
            </button>
          ) : (
            <button className="btn" disabled={planChat.busy} onClick={() => startPlan("ai")}>
              Make it again
            </button>
          )}
          <button className="linkbtn" disabled={planChat.busy} onClick={startOver}>
            Start over
          </button>
        </span>
      </div>
      <Timeline />
    </div>
  );
}

/** The plan chat, on the right side of Day plan. */
export function PlanChat() {
  const { plan, planChat, sendPlan, models, setModel, open, byKey } = useApp();
  const msgs = useRef<HTMLDivElement>(null);
  useEffect(() => {
    msgs.current?.scrollTo({ top: msgs.current.scrollHeight });
  }, [planChat.msgs.length, planChat.busy, planChat.progress.length]);
  const intro = !plan
    ? "Pick one of the two ways on the left. You can also type here, for example “plan my day, but keep the afternoon free”."
    : plan.mode === "self" && !planChat.msgs.length
      ? "Add items from the list under the timeline, or drag them in. I place them around lunch. Change the time with − and +.\n\nWhen you want help, ask me to fill the rest or to check the plan."
      : null;
  return (
    <>
      <div className="msgs" ref={msgs}>
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
          suggestions={plan ? ["Start with the most urgent item", "Is this too much for one day?", "Fill the rest for me"] : ["Plan my day", "Plan my day, keep the afternoon free"]}
          onSend={(text, files) => sendPlan(text, files)}
        />
      </div>
    </>
  );
}
