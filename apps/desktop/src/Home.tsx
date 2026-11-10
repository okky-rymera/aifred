// Home: the brief from AIfred, the day plan in one line, and today's
// missions. The other open items are behind one line. Before the brief
// has missions, the home page shows the list in groups. The box at the
// bottom adds a task or asks AIfred.
import { createContext, useContext, useEffect, useMemo, useState } from "react";
import * as api from "./api";
import { Composer, ICON, Md } from "./Chat";
import { useApp } from "./ctx";
import { GoalTag, GoalsCard } from "./Goals";
import { DayCheckCard, WeekReviewCard } from "./Review";
import { BlockedMark, srcText } from "./Links";
import { WaitingCard } from "./Waiting";
import { HelpCard, useAsking, useHasQuestions } from "./Context";
import { NextStep, stepTaskKey } from "./Steps";
import { progressText } from "./Mission";
import { SRC_NAME, addDays, parentOf, dayOf, dayOfSecs, daysBetween, isItemBlock, dueInfo, dur, fmtTime, groupItems, isPlanRequest, joinLinked, longDay, parseTask, shortDay, treeDepths, type Item } from "./items";

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
  // A tree row can show the reason of a subtask: the buttons correct that subtask.
  const name = i.linked?.find((l) => l.key === i.whyKey)?.title ?? i.title;
  return (
    <div className="why-line">
      <span className="why">
        {i.why} <GoalTag goal={i.goal} ranked />
      </span>
      <span className="fix">
        <button
          className="linkbtn"
          aria-label={`Not today: ${name}`}
          title="This does not need you today. The brief stops counting it for today."
          onClick={() => correctBrief(i, "not_today")}
        >
          Not today
        </button>
        <button
          className="linkbtn"
          aria-label={`Wrong priority: ${name}`}
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

/** True when the rows show their open subtasks as a tree (the Tasks page),
 * false for the one "Subtasks" line (home). TaskGroups sets it for all its lists. */
const TreeRows = createContext(false);

/** The open subtasks of a tree row, each under its parent, with a tick,
 * the source, and the due day. The Tasks page shows this in place of the
 * one "Subtasks" line of the home page. */
function SubTree({ item: i }: { item: Item }) {
  const { links, open, sel } = useApp();
  const linked = i.linked ?? [];
  // Levels count only the lines that show. A finished item does not show,
  // so its open subtasks move up one level, under the nearest open parent.
  const depth = treeDepths(links.links, [i.key, ...linked.map((l) => l.key)]);
  return (
    <ul className="sub-tree" aria-label={`Subtasks of ${i.title}`}>
      {linked.map((l) => (
        <li key={l.key} id={`row-${l.key}`} className={`sub-row${sel === l.key ? " sel" : ""}`} style={{ paddingLeft: (depth.get(l.key) ?? 0) * 16 }}>
          <Check item={l} />
          <button className="linkbtn ttl" title={l.title} aria-label={`Open details: ${l.title}`} onClick={() => open(l.key)}>
            {l.title}
          </button>
          <span className="sub-src">{srcText(l)}</span>
          <DueChip day={l.due} />
        </li>
      ))}
    </ul>
  );
}

function Row(props: { item: Item; why?: boolean; hideToday?: boolean; noActs?: boolean }) {
  const tree = useContext(TreeRows);
  const { sel, open, sched, addToPlan, tomorrow, today, work, startWork, links, byKey } = useApp();
  const i = props.item;
  // A row of linked items is in the plan when one of its items is.
  const keys = [i.key, ...(i.linked ?? []).map((l) => l.key)];
  const at = keys.map((k) => sched.at[k]).find((t) => t !== undefined);
  const inPlan = at !== undefined;
  const acts = !i.done && !props.noActs;
  // "Tomorrow" moves only your task. A linked item due before tomorrow keeps
  // the row where it is, so the button would seem to do nothing.
  const heldByLinked = (i.linked ?? []).some((l) => l.due !== null && l.due < addDays(today, 1));
  const working = !!work && keys.includes(work.key) && !i.done;
  useTick(working);
  // The steps of your task, or of the task that "Suggest steps" fills: an
  // open task directly under this item that you placed there.
  const stepKey = i.task ? undefined : stepTaskKey(i, links.links, byKey);
  const stepTask = i.task ?? (stepKey ? byKey.get(stepKey)?.task : undefined);
  const sub = stepTask?.checklist.length ? `${stepTask.checklist.filter((c) => c.done).length}/${stepTask.checklist.length} steps` : null;
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
        <BlockedMark item={i} />
        {i.linked?.length ? (
          tree ? (
            <SubTree item={i} />
          ) : (
            <div className="linked-src">
              <span>Subtasks</span>
              {i.linked.map((l) => (
                <button key={l.key} className="linkbtn" title={l.title} aria-label={`Open details: ${l.title}`} onClick={() => open(l.key)}>
                  {srcText(l)}
                </button>
              ))}
            </div>
          )
        ) : null}
        {props.why && i.why && !i.done && (
          <>
            <Why item={i} />
            <NextStep item={i} />
          </>
        )}
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
            {i.task && !heldByLinked && (
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
  // A plan from a review only: you did not pick yet how to plan the day.
  const fromReview = plan?.mode === "review" ? planItems.length : 0;
  if (!plan || plan.mode === "review" || !planItems.length) {
    return (
      <div className="strip">
        <span className="ic">{ICON.cal}</span>
        <span className="txt">
          <span>No plan for today yet.</span>
          <small>
            {fromReview > 0 && `Your review put ${fromReview} item${fromReview > 1 ? "s" : ""} in it. `}
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
          Open calendar
        </button>
      </span>
    </div>
  );
}

/** The brief note shows this many lines of moves. A button shows the rest.
 * The first sort after an update can move many old items. */
const MOVES_SHOWN = 5;

/** AIfred's moves of new items into subtasks (sort.rs), as a short note in
 * the brief. Moves to one parent with one reason are one line. Each line has
 * an Undo. The app writes this text from the move log, not the AI. */
function MovesNote(props: { moves: api.Moved[]; more?: number }) {
  const { open, byKey, undoMoves } = useApp();
  const [busy, setBusy] = useState(false);
  const [all, setAll] = useState(false);
  const groups: api.Moved[][] = [];
  for (const m of props.moves) {
    const g = groups.find((x) => x[0].main === m.main && x[0].why === m.why);
    if (g) g.push(m);
    else groups.push([m]);
  }
  if (!groups.length) return null;
  const name = (key: string, title: string) =>
    byKey.has(key) ? (
      <button className="linkbtn" onClick={() => open(key)}>
        {title}
      </button>
    ) : (
      <span>{title}</span>
    );
  const names = (g: api.Moved[]) =>
    g.map((m, i) => (
      <span key={m.id}>
        {i > 0 && (i === g.length - 1 ? (g.length > 2 ? ", and " : " and ") : ", ")}
        {name(m.key, m.title)}
      </span>
    ));
  const undo = async (g: api.Moved[]) => {
    setBusy(true);
    await undoMoves(g);
    setBusy(false);
  };
  const line = (g: api.Moved[], first: boolean) => (
    <>
      {first ? "I made " : ""}
      {names(g)}
      {first ? " a subtask of " : g.length > 1 ? " are now subtasks of " : " is now a subtask of "}
      {name(g[0].main, g[0].main_title)}. {g[0].why}{" "}
      <button className="linkbtn" disabled={busy} aria-label={`Undo: ${g.map((m) => m.title).join(", ")}`} onClick={() => undo(g)}>
        Undo
      </button>
    </>
  );
  return (
    <div className="btext moves" aria-label="What AIfred sorted">
      {groups.length === 1 && groups[0].length === 1 ? (
        <p>{line(groups[0], true)}</p>
      ) : (
        <>
          <p>I reorganized several tasks:</p>
          <ul>
            {(all ? groups : groups.slice(0, MOVES_SHOWN)).map((g) => (
              <li key={g[0].id}>{line(g, false)}</li>
            ))}
          </ul>
          {!all && groups.length > MOVES_SHOWN && (
            <button className="linkbtn" onClick={() => setAll(true)}>
              Show {groups.length - MOVES_SHOWN} more
            </button>
          )}
        </>
      )}
      {!!props.more && (
        <p className="hintx">
          {props.more} older {props.more === 1 ? "move is" : "moves are"} not in this list. Open an item to see its subtasks and AIfred's reasons. "Move out" there takes an item out of a tree.
        </p>
      )}
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
        <>
          <Md className="btext" text={brief.text} onItem={open} hasItem={(k) => byKey.has(k)} />
          {!!brief.moves?.length && <MovesNote moves={brief.moves} more={brief.moves_more} />}
        </>
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
 *   Only "plan my day" opens the Calendar, where AIfred fills the plan.
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
        if (ask) return isPlanRequest(text) ? <>{chip}<span>Opens Calendar</span></> : chip;
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

type Groups = ReturnType<typeof groupItems>;

/** The list in groups, worked out again only when the items, the links, or
 * the day change. `given`: the groups that the caller has already. Then
 * nothing is worked out. */
function useGroups(given?: Groups): Groups {
  const { items, links, today } = useApp();
  const own = useMemo(() => (given ? null : groupItems(joinLinked(items, links.links), today)), [given, items, links.links, today]);
  return given ?? own!;
}

/** The state of the lists: loading, errors, and warnings. */
function ListNotes() {
  const { githubState, clickupState, githubWarning, clickupWarning, mailState, mailWarning, tasksError } = useApp();
  return (
    <>
      {tasksError && <div className="note-line err">Cannot load your tasks: {tasksError}</div>}
      {githubState === "loading" && <div className="note-line">Loading GitHub items…</div>}
      {githubWarning && githubState === "ok" && <div className="note-line warn">{githubWarning}</div>}
      {!["loading", "ok", "off"].includes(githubState) && <div className="note-line err">GitHub: {githubState}</div>}
      {clickupState === "loading" && <div className="note-line">Loading ClickUp tasks…</div>}
      {clickupWarning && clickupState === "ok" && <div className="note-line warn">{clickupWarning}</div>}
      {!["loading", "ok", "off"].includes(clickupState) && <div className="note-line err">ClickUp: {clickupState}</div>}
      {mailWarning && mailState === "ok" && <div className="note-line warn">{mailWarning}</div>}
      {!["loading", "ok", "off"].includes(mailState) && <div className="note-line err">Gmail: {mailState}</div>}
    </>
  );
}

/** The user's raw list in groups: "Needs you now", "Today", "New", "Open",
 * with "Later" and "Finished" behind buttons. Home and the Tasks page show it.
 * `before` goes before the buttons under the groups. `empty` is the text for
 * an empty list. `groups`: the groups when the caller has them already.
 * `noNotes`: the caller shows the state of the lists itself. `tree`: show
 * the open subtasks of each row under it, each level indented. */
export function TaskGroups(props: { before?: React.ReactNode; empty?: string; groups?: Groups; noNotes?: boolean; tree?: boolean }) {
  const { githubState, tasksError } = useApp();
  const [showLater, setShowLater] = useState(false);
  const [showDone, setShowDone] = useState(false);
  const g = useGroups(props.groups);
  const nothing = !g.now.length && !g.today.length && !g.isNew.length && !g.soon.length;
  return (
    <TreeRows.Provider value={!!props.tree}>
      {!props.noNotes && <ListNotes />}
      <Group title="Needs you now" list={g.now} alert why />
      <Group title="Today" list={g.today} hideToday why />
      <Group title="New" list={g.isNew} why />
      <Group title="Open" list={g.soon} />
      {nothing && !tasksError && githubState !== "loading" && (
        <div className="empty">{g.later.length ? "Nothing to do today. Some tasks start later." : (props.empty ?? "Nothing to do. Add a task in the box below.")}</div>
      )}
      <div className="more">
        {props.before}
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
    </TreeRows.Provider>
  );
}

/** One mission of today: the title, the progress, and the reason and the
 * next action of its first open item. A click opens it on the right side,
 * like an item (Mission.tsx). */
function MissionCard({ m }: { m: api.Mission }) {
  const { byKey, work, links, open, openMission, close, selMission } = useApp();
  const show = selMission === m.id;
  // The first item has the next action. When it closed, the next open item leads.
  const leadKey = m.items.find((i) => i.open)?.key;
  const lead = leadKey ? byKey.get(leadKey) : undefined;
  const working = !!work && m.items.some((i) => i.key === work.key);
  useTick(working);
  // The parent of the lead item is the context of the mission: "Part of DEV-123".
  const parentKey = leadKey ? parentOf(links.links, leadKey) : undefined;
  const parent = parentKey && !m.items.some((i) => i.key === parentKey) ? byKey.get(parentKey) : undefined;
  const p = m.progress;
  const toggle = () => (show ? close() : openMission(m.id));
  return (
    <li className={`row mission${show ? " sel" : ""}`} id={`mission-${m.id}`}>
      <span className="m-pct" title={progressText(p)} aria-hidden>
        {p.percent}%
      </span>
      <div className="t-col">
        <div
          className="t-main"
          role="button"
          tabIndex={0}
          aria-expanded={show}
          aria-label={`${show ? "Close" : "Open"} mission: ${m.title}`}
          onClick={toggle}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              toggle();
            }
          }}
        >
          <span className="t-title">{m.title}</span>
          <div className="meta">
            <span title={progressText(p)}>{p.percent}% done</span>
            <span>
              {m.items.length} item{m.items.length === 1 ? "" : "s"}
            </span>
            {m.checklist.length > 0 && (
              <span>
                {p.steps_done}/{p.steps} steps
              </span>
            )}
            {m.doer === "aifred" && <span className="tagc">AIfred does the next step</span>}
            {working && <span className="planned working">Working {dur(workMinutes(work!))}</span>}
          </div>
        </div>
        {parent && (
          <div className="part-of">
            Part of{" "}
            <button className="linkbtn" aria-label={`Open details: ${parent.title}`} onClick={() => open(parent.key)}>
              {parent.title}
            </button>
          </div>
        )}
        {lead && <BlockedMark item={lead} />}
        {lead?.why && (
          <div className="why-line">
            <span className="why">
              {lead.why} <GoalTag goal={lead.goal} ranked />
            </span>
          </div>
        )}
        {lead && <NextStep item={lead} />}
      </div>
      <span />
    </li>
  );
}

/** Today's missions, the urgent items in no mission, and the other open
 * items behind one closed line. */
function Missions({ missions, more }: { missions: api.Mission[]; more: React.ReactNode }) {
  const { items, links, today } = useApp();
  const [showOther, setShowOther] = useState(false);
  // The rows are built again from the items that are in no mission: the
  // missions show the others. So each tree row gets its due day, priority,
  // reason, and next action only from the items that it still shows. An
  // urgent subtask in no mission stays in "Needs you now", also when the
  // top item of its tree is in a mission: it is then the row.
  const other = useMemo(() => {
    const inMission = new Set(missions.flatMap((m) => m.items.map((i) => i.key)));
    return groupItems(
      joinLinked(
        items.filter((i) => !inMission.has(i.key)),
        links.links,
      ),
      today,
    );
  }, [missions, items, links.links, today]);
  // "Needs you now" always shows: an urgent item must not hide behind the line.
  const hidden = useMemo(() => ({ ...other, now: [] }), [other]);
  const n = other.today.length + other.isNew.length + other.soon.length + other.later.length;
  return (
    <>
      <ListNotes />
      <div className="group-h">
        Today's missions <span className="c">{missions.length}</span>
      </div>
      <ul className="list card-list" aria-label="Today's missions">
        {missions.map((m) => (
          <MissionCard key={m.id} m={m} />
        ))}
      </ul>
      <Group title="Needs you now" list={other.now} alert why />
      <div className="more">
        <button className="linkbtn" aria-expanded={showOther} onClick={() => setShowOther((s) => !s)} title="The open items that are in no mission today">
          {showOther ? "Hide" : "Show"} other open items ({n})
        </button>
        {more}
      </div>
      {showOther && <TaskGroups groups={hidden} noNotes empty="No other open items." />}
    </>
  );
}

export function Home() {
  const { reviewCards, setReviewCard, today, goTasks, brief, briefState, settings, homeLoading } = useApp();
  const g = useGroups();
  const missions = brief?.missions ?? [];
  // The missions with an item that you put there yourself. They show also
  // while AIfred asks its questions.
  const ownMissions = missions.filter((m) => m.items.some((i) => i.by === "you"));
  // AIfred has questions, and the first brief of the day holds its
  // missions for them (brief.rs): the page shows only them, and the
  // missions with items that you put there yourself. The brief, the other
  // missions, the goals, the other cards, and the chat box wait for the
  // answers, so that you see the brief and missions that AIfred makes with them.
  const asking = useAsking();
  // A later brief of the day does not hold its missions: the questions show
  // in a card above them.
  const questions = useHasQuestions();
  // AIfred writes today's first brief (first it looks up the tasks that it
  // does not know), or writes again the brief that held its missions for
  // the questions. There are no missions to show yet, and the list of all
  // tasks would be the wrong page: a card says that AIfred works. A later
  // write of the day keeps the page: you can still use the chat box.
  const preparing =
    !asking &&
    (brief?.day !== today || !!brief?.held) &&
    (briefState === "writing" || (!!settings.auto_brief && brief?.day !== today && briefState === "idle"));
  // Before the first loads come back, the page would show the task list
  // for a moment, and then the questions.
  const loading = !asking && homeLoading && briefState !== "writing";
  const more = (
    <>
      <button className="linkbtn" onClick={() => setReviewCard("day", "open")} title="What you finished today, and what moves to tomorrow">
        End the day
      </button>
      <button className="linkbtn" onClick={() => setReviewCard("week", "open")} title="Check your goals and the items that did not move for 3 weeks">
        Weekly review
      </button>
      <button className="linkbtn" onClick={goTasks} title="Your tasks and AIfred's tasks">
        See all tasks
      </button>
    </>
  );
  const hour = new Date().getHours();
  const hello = hour < 12 ? "Good morning." : hour < 18 ? "Good afternoon." : "Good evening.";
  return (
    <div className="home-main">
      <div className="scroll">
        <div className="page-inner">
          <h2 className="hello">
            {hello} <span>{longDay(today)}</span>
          </h2>
          {asking ? (
            <>
              <HelpCard />
              {/* While the brief holds, it keeps only the missions with
                  items that you put there yourself (brief.rs). */}
              {ownMissions.length > 0 && <Missions missions={ownMissions} more={null} />}
            </>
          ) : loading ? (
            <section className="waiting prep" aria-label="Loading your day">
              <div className="prep-row" role="status">
                <span className="prep-dot" aria-hidden="true" />
                Loading your day…
              </div>
            </section>
          ) : preparing ? (
            <section className="waiting prep" aria-label="Preparing your day">
              <div className="eyebrow">
                <span>✦ Preparing your day</span>
              </div>
              <div className="prep-row" role="status">
                <span className="prep-dot" aria-hidden="true" />
                AIfred looks up your tasks, then writes today's brief and missions. This can take a few minutes.
              </div>
            </section>
          ) : (
            <>
              {questions && <HelpCard />}
              <WorkBar />
              {reviewCards.day !== "hidden" && <DayCheckCard force={reviewCards.day === "open"} onClose={() => setReviewCard("day", "hidden")} />}
              <GoalsCard />
              <BriefCard needYou={g.now.length + g.today.length} />
              {reviewCards.week !== "hidden" && <WeekReviewCard force={reviewCards.week === "open"} onClose={() => setReviewCard("week", "hidden")} />}
              <WaitingCard />
              {/* Before the brief makes missions, the list in groups. */}
              {missions.length > 0 ? <Missions missions={missions} more={more} /> : <TaskGroups groups={g} before={more} />}
            </>
          )}
        </div>
      </div>
      {/* The box comes back after the questions: you answer them first. */}
      {!asking && !preparing && !loading && (
        <div className="a-foot">
          <HomeBox />
        </div>
      )}
    </div>
  );
}
