// Home: the brief from AIfred, the day plan in one line, and the list in
// groups. The box at the bottom adds a task or asks AIfred.
import { useState } from "react";
import * as api from "./api";
import { Composer, ICON, Md } from "./Chat";
import { useApp } from "./ctx";
import { SRC_NAME, dueInfo, dur, fmtTime, groupItems, longDay, parseQuick, shortDay, type Item } from "./items";

export function Check(props: { item: Item }) {
  const { toggleDone } = useApp();
  const i = props.item;
  // GitHub items have no tick: AIfred only reads GitHub. An item leaves the list when it closes there.
  if (i.src === "gh") return <span className="chk-none" title="GitHub is read-only. The item leaves the list when it closes on GitHub." />;
  const label = i.src === "cu" ? `Close in ClickUp: ${i.title}` : `${i.done ? "Open again" : "Finish"}: ${i.title}`;
  return (
    <button
      className={`chk p-${i.priority ?? "normal"}`}
      aria-pressed={i.done}
      aria-label={label}
      title={i.src === "cu" ? "Close this task in ClickUp" : undefined}
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

export function DueChip({ day }: { day: string | null }) {
  const { today } = useApp();
  if (!day) return null;
  const d = dueInfo(day, today);
  return <span className={`due ${d.cls}`}>{d.text}</span>;
}

function Row(props: { item: Item; why?: boolean; hideToday?: boolean; noActs?: boolean }) {
  const { sel, open, sched, addToPlan, tomorrow, today } = useApp();
  const i = props.item;
  const at = sched.at[i.key];
  const inPlan = at !== undefined;
  const acts = !i.done && !props.noActs;
  const sub = i.task?.checklist.length ? `${i.task.checklist.filter((c) => c.done).length}/${i.task.checklist.length} steps` : null;
  return (
    <li className={`row${i.done ? " done" : ""}${sel === i.key ? " sel" : ""}`} id={`row-${i.key}`}>
      <Check item={i} />
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
        {props.why && i.why && !i.done && <span className="why">{i.why}</span>}
      </div>
      <span className={`r-side${acts ? " has-acts" : ""}`}>
        <span className="r-info">
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
  const next = sched.all.find((b) => !b.lunch && !byKey.get(b.key)?.done);
  const span = Math.max(1, (sched.end ?? 0) - (sched.all[0]?.start ?? 0));
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
          {sched.all.map((b) => (
            <i
              key={b.key}
              className={!b.lunch && byKey.get(b.key)?.done ? "d" : ""}
              style={{ width: `${(((b.end - b.start) / span) * 100).toFixed(1)}%`, background: b.lunch ? "var(--line)" : `var(--src-${byKey.get(b.key)?.src ?? "me"})` }}
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
        <div className="btext wait">AIfred is reading your tasks, GitHub, and ClickUp to write today's brief…</div>
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

/** The home box: a new task, or a question to AIfred. The chip shows which, and a click on it changes it. */
function HomeBox() {
  const { today, main, sendMain, addTask, models, setModel, startPlan } = useApp();
  const [force, setForce] = useState<"ask" | "task" | null>(null);

  function read(text: string) {
    const p = parseQuick(text, today);
    if (!p || !force) return p;
    if (force === "ask") return { ask: true as const, text: text.trim(), plan: false };
    if (p.ask) return { ask: false as const, title: text.trim(), due: null, priority: null, tags: [] };
    return p;
  }

  return (
    <Composer
      id="main-home"
      placeholder="Add a task or ask AIfred"
      sendLabel="Send"
      // Adding a task needs no AI call, so only a question waits for the last answer.
      busy={(text, files) => main.busy && (read(text) ?? { ask: files.length > 0 }).ask}
      model={models.main}
      onModel={(m) => setModel("main", m)}
      hint={(text, files) => {
        const p = read(text);
        if (!p) return null;
        const flip = (
          <button className={`chip${p.ask ? " ask" : ""}`} title="Click to change" onClick={() => setForce(p.ask ? "task" : "ask")}>
            {p.ask ? (p.plan ? "Plan my day" : "Ask AIfred") : "Add task"}
          </button>
        );
        if (p.ask) return <>{flip}<span>{p.plan ? "Opens Day plan" : "AIfred answers"}</span></>;
        return (
          <>
            {flip}
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
        const p = read(text) ?? (files.length ? { ask: true as const, text: "Look at these files.", plan: false } : null);
        setForce(null);
        if (!p) return false;
        if (p.ask && p.plan) {
          startPlan("ai");
          return true;
        }
        if (p.ask) return sendMain(p.text, files);
        return addTask({ title: p.title, due: p.due ?? undefined, priority: p.priority ?? undefined, tags: p.tags, files: files.map((f) => f.id) });
      }}
    />
  );
}

export function Home() {
  const { items, today, githubState, clickupState, githubWarning, clickupWarning, tasksError } = useApp();
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
          <BriefCard needYou={g.now.length + g.today.length} />
          {tasksError && <div className="note-line err">Cannot load your tasks: {tasksError}</div>}
          {githubState === "loading" && <div className="note-line">Loading GitHub items…</div>}
          {githubWarning && githubState === "ok" && <div className="note-line warn">{githubWarning}</div>}
          {!["loading", "ok", "off"].includes(githubState) && <div className="note-line err">GitHub: {githubState}</div>}
          {clickupState === "loading" && <div className="note-line">Loading ClickUp tasks…</div>}
          {clickupWarning && clickupState === "ok" && <div className="note-line warn">{clickupWarning}</div>}
          {!["loading", "ok", "off"].includes(clickupState) && <div className="note-line err">ClickUp: {clickupState}</div>}
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
