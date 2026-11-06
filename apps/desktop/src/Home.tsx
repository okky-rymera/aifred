// Home: the brief from AIfred, the day plan in one line, and the list in
// groups. The box at the bottom adds a task or asks AIfred.
import { useEffect, useState } from "react";
import * as api from "./api";
import { Composer, ICON, Md } from "./Chat";
import { useApp } from "./ctx";
import { GoalTag, GoalsCard } from "./Goals";
import { WaitingCard } from "./Waiting";
import { SRC_NAME, dayOf, dayOfSecs, daysBetween, isItemBlock, dueInfo, dur, fmtTime, groupItems, isPlanRequest, longDay, parseTask, shortDay, type Item } from "./items";

export function Check(props: { item: Item }) {
  const { toggleDone } = useApp();
  const i = props.item;
  // GitHub items have no tick: AIfred only reads GitHub. An item leaves the list when it closes there.
  if (i.src === "gh") return <span className="chk-none" title="GitHub is read-only. The item leaves the list when it closes on GitHub." />;
  const label = i.src === "cu" ? `Close in ClickUp: ${i.title}` : i.src === "gm" ? `No answer needed: ${i.title}` : `${i.done ? "Open again" : "Finish"}: ${i.title}`;
  const tip = i.src === "cu" ? "Close this task in ClickUp" : i.src === "gm" ? "No answer needed: hide this email until a new message comes. Gmail does not change." : undefined;
  return (
    <button
      className={`chk p-${i.priority ?? "normal"}`}
      aria-pressed={i.done}
      aria-label={label}
      title={tip}
      onClick={(e) => {
        e.stopPropagation();
        toggleDone(i);
      }}
    />
  );
}

export function PriPill({ p }: { p: api.Priority | null }) {
  if (p === "urgent") return <span className="pill urgent">Urgent</span>;
  if (p === "high") return <span className="pill high">High</span>;
  if (p === "low") return <span className="pill">Low</span>;
  return null;
}

const REASON: Record<api.GithubItem["reason"], string> = { review_requested: "Review", assigned: "Assigned", your_pr: "Your PR" };

/** Where the item is from, its tags, and for GitHub why it is in the list. */
export function SrcLine({ item: i, extra }: { item: Item; extra?: React.ReactNode }) {
  return (
    <div className="meta">
      <span className={`src ${i.src}`}>
        {SRC_NAME[i.src]}
        {i.ref && ` · ${i.ref}`}
      </span>
      {i.gh && <span className="tagc">{REASON[i.gh.reason]}</span>}
      {i.mail && <MailTag m={i.mail} />}
      {i.cu && (
        <span className="pill status" title={`ClickUp status in ${i.cu.workspace}`}>
          <i style={{ background: i.cu.status_color || "var(--faint)" }} aria-hidden />
          {i.cu.status}
        </span>
      )}
      {i.tags.map((t) => (
        <span key={t} className="tagc">
          #{t}
        </span>
      ))}
      {extra}
    </div>
  );
}

/** How long an email waits for your answer. */
function MailTag({ m }: { m: api.Mail }) {
  const { today } = useApp();
  // Calendar days, as in the "since" day: a mail from 23:00 yesterday waits 1 day at 08:00.
  const days = Math.max(0, daysBetween(dayOf(new Date(m.since * 1000)), today));
  return (
    <span className={`tagc${days >= 2 ? " late" : ""}`} title={`Waits for your answer since ${dayOfSecs(m.since, today)}`}>
      {m.unread ? "Unread · " : ""}
      {days < 1 ? "Waits since today" : `Waits ${days} day${days > 1 ? "s" : ""}`}
    </span>
  );
}

export function DueChip({ day }: { day: string | null }) {
  const { today } = useApp();
  if (!day) return null;
  const d = dueInfo(day, today);
  return <span className={`due ${d.cls}`}>{d.text}</span>;
}

/** The reason from the brief, and two buttons to correct it. AIfred learns from the corrections. */
function Why({ item: i }: { item: Item }) {
  const { correctBrief } = useApp();
  return (
    <div className="why-line">
      <span className="why">
        {i.why} <GoalTag goal={i.goal} ranked />
      </span>
      <span className="fix">
        <button
          className="linkbtn"
          aria-label={`Not today: ${i.title}`}
          title="This does not need you today. The brief stops counting it for today."
          onClick={() => correctBrief(i, "not_today")}
        >
          Not today
        </button>
        <button
          className="linkbtn"
          aria-label={`Wrong priority: ${i.title}`}
          title="The brief ranked this too high. AIfred ranks it lower from now on, and learns from repeated corrections."
          onClick={() => correctBrief(i, "wrong_priority")}
        >
          Wrong priority
        </button>
      </span>
    </div>
  );
}

/** Minutes of work on the running item: the earlier sessions and this one. */
export function workMinutes(w: api.Running, nowSecs = Date.now() / 1000): number {
  return Math.max(0, Math.round((w.before + nowSecs - w.started) / 60));
}

/** Draws the parent again every 15 seconds while `on` is true, so a timer moves. */
function useTick(on: boolean) {
  const [, setN] = useState(0);
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => setN((n) => n + 1), 15_000);
    return () => clearInterval(t);
  }, [on]);
}

function Row(props: { item: Item; why?: boolean; hideToday?: boolean; noActs?: boolean }) {
  const { sel, open, sched, addToPlan, tomorrow, today, work, startWork } = useApp();
  const i = props.item;
  const at = sched.at[i.key];
  const inPlan = at !== undefined;
  const acts = !i.done && !props.noActs;
  const working = work?.key === i.key && !i.done;
  useTick(working);
  const sub = i.task?.checklist.length ? `${i.task.checklist.filter((c) => c.done).length}/${i.task.checklist.length} steps` : null;
  return (
    <li className={`row${i.done ? " done" : ""}${sel === i.key ? " sel" : ""}`} id={`row-${i.key}`}>
      <Check item={i} />
      <div className="t-col">
        <div
          className="t-main"
          role="button"
          tabIndex={0}
          aria-label={`Open details: ${i.title}`}
          onClick={() => open(i.key)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              open(i.key);
            }
          }}
        >
          <span className="t-title">{i.title}</span>
          <SrcLine item={i} extra={sub && <span>{sub}</span>} />
        </div>
        {props.why && i.why && !i.done && <Why item={i} />}
      </div>
      <span className={`r-side${acts ? " has-acts" : ""}`}>
        <span className="r-info">
          {working && (
            <span className="planned working" title="You work on this now">
              Working {dur(workMinutes(work!))}
            </span>
          )}
          {inPlan ? (
            <span className="planned" title="In today's plan">
              Planned {fmtTime(at)}
            </span>
          ) : props.hideToday && i.due === today ? null : (
            <DueChip day={i.due} />
          )}
          {i.start && i.start > today && <span className="due">From {shortDay(i.start, today)}</span>}
          <PriPill p={i.priority} />
        </span>
        {acts && (
          <span className="row-acts">
            {!working && (
              <button className="x" title="Start work on this. AIfred keeps the real time, to make better time guesses." onClick={() => startWork(i.key)}>
                Start
              </button>
            )}
            {!inPlan && (
              <button className="x" title="Add to today's plan" onClick={() => addToPlan(i.key)}>
                + Plan
              </button>
            )}
            {i.task && (
              <button className="x" title="Move to tomorrow" onClick={() => tomorrow(i)}>
                Tomorrow
              </button>
            )}
          </span>
        )}
      </span>
    </li>
  );
}

function Group(props: { title: string; list: Item[]; alert?: boolean; why?: boolean; hideToday?: boolean }) {
  if (!props.list.length) return null;
  return (
    <>
      <div className={`group-h${props.alert ? " alert" : ""}`}>
        {props.title} <span className="c">{props.list.length}</span>
      </div>
      <ul className="list card-list" aria-label={props.title}>
        {props.list.map((i) => (
          <Row key={i.key} item={i} why={props.why} hideToday={props.hideToday} />
        ))}
      </ul>
    </>
  );
}

/** The item that you work on now: the time so far, the guess, Stop, and Done. */
function WorkBar() {
  const { work, byKey, stopWork, finishItem, open } = useApp();
  useTick(!!work);
  if (!work) return null;
  const i = byKey.get(work.key);
  const m = workMinutes(work);
  return (
    <section className="workbar" aria-label="Working on">
      <span className="dot" aria-hidden />
      <span className="txt">
        <span>
          Working on{" "}
          <button className="linkbtn ttl" onClick={() => open(work.key)}>
            {i?.title ?? work.title}
          </button>
        </span>
        <small>
          {dur(m)}
          {/* An item with no guess has the default time in `minutes`, which nobody guessed. */}
          {i && i.minutesBy !== null ? ` of ${dur(i.minutes)} guessed${m > i.minutes ? `, ${dur(m - i.minutes)} over` : ""}` : ""}
        </small>
      </span>
      <span className="btns">
        <button className="btn" onClick={stopWork} title="Stop the timer. The item stays open.">
          Stop
        </button>
        {!i?.done && (
          <button
            className="btn pri"
            onClick={() => finishItem(work.key)}
            title={!i ? "The work is done. The item is not in the list now." : i.gh ? "The work is done. The item leaves the list when it closes on GitHub." : "The work is done"}
          >
            Done
          </button>
        )}
      </span>
    </section>
  );
}

/** The day plan in one line: the next task, the end of the day, and a bar. */
function PlanStrip() {
  const { plan, planItems, sched, byKey, settings, startPlan, goPlan } = useApp();
  if (!plan || !planItems.length) {
    return (
      <div className="strip">
        <span className="ic">{ICON.cal}</span>
        <span className="txt">
          <span>No plan for today yet.</span>
          <small>
            AIfred can put your tasks in order and place them around lunch{settings.lunch_minutes ? ` at ${settings.lunch_start}` : ""}.
          </small>
        </span>
        <span className="btns">
          <button className="btn pri" onClick={() => startPlan("ai")}>
            Plan it for me
          </button>
          <button className="btn" onClick={() => startPlan("self")}>
            I plan it myself
          </button>
        </span>
      </div>
    );
  }
  const next = sched.all.find((b) => isItemBlock(b) && !byKey.get(b.key)?.done);
  // The bar shows the time from the first block to the end of the planned
  // work. Each block has its own place on that time line, so meetings that
  // overlap each other or lunch do not push the later blocks.
  const from = sched.all[0]?.start ?? 0;
  const to = sched.end ?? from;
  const span = Math.max(1, to - from);
  const pct = (m: number) => `${((m / span) * 100).toFixed(1)}%`;
  return (
    <div className="strip">
      <span className="ic">{ICON.cal}</span>
      <span className="txt">
        <span>
          {next ? (
            <>
              Next: <b>{byKey.get(next.key)?.title}</b> at {fmtTime(next.start)}
            </>
          ) : (
            "All planned tasks are finished."
          )}
        </span>
        <small>
          {plan.mode === "ai" ? "AIfred made this plan" : "You made this plan"} · ends {fmtTime(sched.end ?? 0)} ·{" "}
          {sched.free < 0 ? `${dur(-sched.free)} too much` : `${dur(sched.free)} free`}
        </small>
        <span className="bar" aria-hidden>
          {sched.all.filter((b) => b.start < to).map((b) => (
            <i
              key={b.key}
              className={isItemBlock(b) && byKey.get(b.key)?.done ? "d" : ""}
              title={b.meeting?.title}
              style={{
                left: pct(b.start - from),
                width: pct(Math.min(b.end, to) - b.start),
                background: b.lunch ? "var(--line)" : b.meeting ? "var(--muted)" : `var(--src-${byKey.get(b.key)?.src ?? "me"})`,
              }}
            />
          ))}
        </span>
      </span>
      <span className="btns">
        <button className="btn" onClick={goPlan}>
          Open plan
        </button>
      </span>
    </div>
  );
}

function BriefCard(props: { needYou: number }) {
  const { brief, briefState, writeBrief, open, byKey, sendMain, main } = useApp();
  const written = brief ? new Date(brief.written * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : "";
  return (
    <section className="brief" aria-label="Brief">
      <div className="eyebrow">
        <span>✦ Brief</span>
        {brief && <span>· written {written}</span>}
        <span className="sp" />
        {brief && (
          <button className="linkbtn" disabled={main.busy} onClick={() => sendMain("What changed since the brief this morning?", [])}>
            What changed?
          </button>
        )}
        <button className="linkbtn" disabled={briefState === "writing"} onClick={writeBrief}>
          {briefState === "writing" ? "Writing…" : brief ? "Write again" : "Write the brief"}
        </button>
      </div>
      {brief ? (
        <Md className="btext" text={brief.text} onItem={open} hasItem={(k) => byKey.has(k)} />
      ) : briefState === "writing" ? (
        <div className="btext wait">AIfred is reading your tasks, GitHub, ClickUp, and email to write today's brief…</div>
      ) : (
        <div className="btext wait">
          {props.needYou ? `${props.needYou} things need you today. ` : "Nothing urgent is due today. "}
          AIfred has not written today's brief yet.
        </div>
      )}
      {briefState !== "idle" && briefState !== "writing" && <div className="d-err">Cannot write the brief: {briefState}</div>}
      <div className="acts">
        <button className="sug" disabled={main.busy} onClick={() => sendMain("Which pull requests wait for my review? One line each.", [])}>
          What should I review?
        </button>
        <button className="sug" disabled={main.busy} onClick={() => sendMain("What is the most urgent thing on my list, and what is the next step?", [])}>
          What is most urgent?
        </button>
      </div>
      <PlanStrip />
    </section>
  );
}

/** The home box. Tab switches the mode:
 * - "Ask AIfred": the message goes to AIfred. AIfred answers, adds a task
 *   ("pay rent friday high #home"), or makes a job ("remind me every Monday").
 *   Only "plan my day" opens the Day plan, where AIfred fills the plan.
 * - "Add task": the line is a new task at once, with no AI call. The day, the
 *   priority, and the #tags come from the words. */
function HomeBox() {
  const { today, main, sendMain, addTask, models, setModel, startPlan, homeMode: mode, setHomeMode: setMode } = useApp();
  const flip = () => setMode(mode === "ask" ? "task" : "ask");
  const ask = mode === "ask";

  return (
    <Composer
      id="main-home"
      label="Add a task or ask AIfred"
      placeholder={ask ? "Ask AIfred" : "Add a task"}
      sendLabel={ask ? "Send" : "Add"}
      tone={ask ? undefined : "task"}
      // Adding a task needs no AI call, so it does not wait for an answer.
      // A task needs a name: files alone do not make a task, so Add stays off.
      busy={(text) => (ask ? main.busy : !parseTask(text, today))}
      // A new task uses no model, so the task mode has no model picker.
      model={models.main}
      onModel={ask ? (m) => setModel("main", m) : undefined}
      onTab={flip}
      hint={(text, files) => {
        const chip = (
          <button className={`chip ${ask ? "ask" : "task"}`} title="Click or press Tab to change. To leave the box, press Esc, then Tab." onClick={flip}>
            {ask ? "Ask AIfred" : "Add task"}
          </button>
        );
        if (ask) return isPlanRequest(text) ? <>{chip}<span>Opens Day plan</span></> : chip;
        const p = parseTask(text, today);
        if (!p) return files.length ? <>{chip}<span>Type a task name to add the files to it</span></> : chip;
        return (
          <>
            {chip}
            <b>{p.title}</b>
            {p.due && <span className="chip">{dueInfo(p.due, today).text}</span>}
            {p.priority && <span className="chip">{p.priority[0].toUpperCase() + p.priority.slice(1)}</span>}
            {p.tags.map((t) => (
              <span key={t} className="chip">
                #{t}
              </span>
            ))}
            {files.length > 0 && <span className="chip">{files.length} file{files.length > 1 ? "s" : ""}</span>}
          </>
        );
      }}
      onSend={async (text, files) => {
        if (!ask) {
          const p = parseTask(text, today);
          if (!p) return false;
          return addTask({ title: p.title, due: p.due ?? undefined, priority: p.priority ?? undefined, tags: p.tags, files: files.map((f) => f.id) });
        }
        const t = text.trim() || (files.length ? "Look at these files." : "");
        if (!t) return false;
        if (files.length === 0 && isPlanRequest(t)) {
          startPlan("ai");
          return true;
        }
        return sendMain(t, files);
      }}
    />
  );
}

export function Home() {
  const { items, today, githubState, clickupState, githubWarning, clickupWarning, mailState, mailWarning, tasksError } = useApp();
  const [showLater, setShowLater] = useState(false);
  const [showDone, setShowDone] = useState(false);
  const g = groupItems(items, today);
  const hour = new Date().getHours();
  const hello = hour < 12 ? "Good morning." : hour < 18 ? "Good afternoon." : "Good evening.";
  const nothing = !g.now.length && !g.today.length && !g.isNew.length && !g.soon.length;
  return (
    <div className="home-main">
      <div className="scroll">
        <div className="page-inner">
          <h2 className="hello">
            {hello} <span>{longDay(today)}</span>
          </h2>
          <WorkBar />
          <GoalsCard />
          <BriefCard needYou={g.now.length + g.today.length} />
          <WaitingCard />
          {tasksError && <div className="note-line err">Cannot load your tasks: {tasksError}</div>}
          {githubState === "loading" && <div className="note-line">Loading GitHub items…</div>}
          {githubWarning && githubState === "ok" && <div className="note-line warn">{githubWarning}</div>}
          {!["loading", "ok", "off"].includes(githubState) && <div className="note-line err">GitHub: {githubState}</div>}
          {clickupState === "loading" && <div className="note-line">Loading ClickUp tasks…</div>}
          {clickupWarning && clickupState === "ok" && <div className="note-line warn">{clickupWarning}</div>}
          {!["loading", "ok", "off"].includes(clickupState) && <div className="note-line err">ClickUp: {clickupState}</div>}
          {mailWarning && mailState === "ok" && <div className="note-line warn">{mailWarning}</div>}
          {!["loading", "ok", "off"].includes(mailState) && <div className="note-line err">Gmail: {mailState}</div>}
          <Group title="Needs you now" list={g.now} alert why />
          <Group title="Today" list={g.today} hideToday why />
          <Group title="New" list={g.isNew} why />
          <Group title="Open" list={g.soon} />
          {nothing && !tasksError && githubState !== "loading" && (
            <div className="empty">{g.later.length ? "Nothing to do today. Some tasks start later." : "Nothing to do. Add a task in the box below."}</div>
          )}
          <div className="more">
            {g.later.length > 0 && (
              <button className="linkbtn" onClick={() => setShowLater((s) => !s)} title="Tasks with a start day after today">
                {showLater ? "Hide" : "Show"} later ({g.later.length})
              </button>
            )}
            {g.done.length > 0 && (
              <button className="linkbtn" onClick={() => setShowDone((s) => !s)}>
                {showDone ? "Hide" : "Show"} finished ({g.done.length})
              </button>
            )}
          </div>
          {showLater && g.later.length > 0 && (
            <ul className="list card-list" aria-label="Later">
              {g.later.map((i) => (
                <Row key={i.key} item={i} />
              ))}
            </ul>
          )}
          {showDone && g.done.length > 0 && (
            <ul className="list card-list" aria-label="Finished" style={{ marginTop: 8 }}>
              {g.done.map((i) => (
                <Row key={i.key} item={i} noActs />
              ))}
            </ul>
          )}
        </div>
      </div>
      <div className="a-foot">
        <HomeBox />
      </div>
    </div>
  );
}
