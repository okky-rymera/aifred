import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "./api";
import { Composer, ICON, Messages, type ChatMsg } from "./Chat";
import { Ctx, openQuestions, useApp, type AppCtx, type CardMode, type ChatState, type ListState, type Talk } from "./ctx";
import { Detail } from "./Detail";
import { MissionDetail, missionChatKey, sideThread } from "./Mission";
import { Home } from "./Home";
import { WEEK, addDays, buildItems, dayName, dayOf, dur, eventsOn, fmtTime, schedule, shortDay, type DayEvents, type Item, type Schedule } from "./items";
import { TasksPage } from "./Tasks";
import { MemoryList } from "./Memory";
import { CalendarView, PlanChat } from "./Calendar";
import { RestartButton, SettingsPage } from "./Settings";
import { useItemsSorted, useTasksChanged } from "./useTasksChanged";

type Page = "home" | "plan" | "tasks" | "memory" | "settings";
type Toast = { text: string; undo?: () => void; error?: boolean };
/** A task that was just closed in ClickUp. The window keeps each one with
 * its own undo: a second close must not lose the undo of the first. */
type Closed = { id: string; title: string; before: string };

const EMPTY_CHAT: ChatState = { msgs: [], busy: false, progress: [] };

/** Today, as "YYYY-MM-DD". It changes at midnight. A check each minute also works after the computer sleeps. */
function useToday(): string {
  const [day, setDay] = useState(() => dayOf(new Date()));
  useEffect(() => {
    const timer = setInterval(() => setDay(dayOf(new Date())), 60_000);
    return () => clearInterval(timer);
  }, []);
  return day;
}

/** "5m ago" for a time in Unix seconds. */
function agoSecs(t: number): string {
  const s = Date.now() / 1000 - t;
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

/** A value that this window keeps for you on this computer. It can be missing. */
function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function store(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* the window works without it */
  }
}

/** The model argument of one message: the pick, or "default" for the default model. */
const pickArg = (m: string | null) => m ?? "default";

/** A message with files and no text asks AIfred to look at the files. */
const withFiles = (text: string, files: unknown[]) => (text.trim() || !files.length ? text : "Look at these files.");

/** Puts an email back in its list, newest first (for Undo). */
const putBack = (list: api.Mail[], m: api.Mail) => [m, ...list.filter((x) => x.id !== m.id)].sort((a, b) => b.last_at - a.last_at);

/** The real time of a finished item for a toast, with a space first:
 * " It took 12 m, the guess was 10 m." A forgotten timer (more than 10
 * hours) does not count, and the text says so. "" for no work time. */
const tookText = (d: api.WorkDone | null) => {
  if (!d) return "";
  const took = d.minutes ? ` It took ${dur(d.minutes)}${d.guess ? `, the guess was ${dur(d.guess)}` : ""}.` : "";
  const skipped = !d.skipped
    ? ""
    : d.skipped === 1
      ? " One timer ran for more than 10 hours, so AIfred did not count it."
      : ` ${d.skipped} timers ran for more than 10 hours, so AIfred did not count them.`;
  return took + skipped;
};

const turnMsgs = (turns: (api.Turn | api.SideTurn)[]): ChatMsg[] =>
  turns.map((t) => ({ role: t.role, text: t.text, files: t.files, info: "info" in t ? t.info : undefined }));

export default function App() {
  const today = useToday();
  const [page, setPage] = useState<Page>("home");
  const [info, setInfo] = useState<api.Info | null>(null);
  const [loadError, setLoadError] = useState("");
  const [facts, setFacts] = useState<api.Fact[]>([]);
  // The Google connection, read once here for all item panels. The Google
  // part of Settings reads it again after each change.
  const [google, setGoogle] = useState<api.GoogleStatus | null>(null);
  const loadGoogle = useCallback(() => {
    api.googleStatus().then(setGoogle).catch(() => {});
  }, []);
  useEffect(loadGoogle, [loadGoogle]);
  const [tasks, setTasks] = useState<api.Task[]>([]);
  const [tasksError, setTasksError] = useState("");
  const [tasksLoaded, setTasksLoaded] = useState(false);
  const [github, setGithub] = useState<api.GithubList>({ items: [], fetched_at: 0 });
  const [githubState, setGithubState] = useState<ListState>("loading");
  const [clickup, setClickup] = useState<api.ClickupList>({ items: [], fetched_at: 0 });
  const [clickupState, setClickupState] = useState<ListState>("loading");
  // True after the first load of each list came back (also with an error).
  // A later reload must not make the home page wait again (homeLoading).
  const [githubBack, setGithubBack] = useState(false);
  const [clickupBack, setClickupBack] = useState(false);
  useEffect(() => {
    if (githubState !== "loading") setGithubBack(true);
  }, [githubState]);
  useEffect(() => {
    if (clickupState !== "loading") setClickupBack(true);
  }, [clickupState]);
  const [calendar, setCalendar] = useState<api.CalendarList>({ events: [], fetched_at: 0 });
  const [calendarState, setCalendarState] = useState<ListState>("loading");
  const [mail, setMail] = useState<api.MailList>({ reply: [], sent: [], fetched_at: 0 });
  const [mailState, setMailState] = useState<ListState>("loading");
  const [waiting, setWaiting] = useState<api.Waiting[]>([]);
  const [waitingError, setWaitingError] = useState("");
  const [clickupToken, setClickupToken] = useState<api.ClickupTokenStatus | null>(null);
  const [settings, setSettings] = useState<api.Settings | null>(null);
  const [update, setUpdate] = useState<api.UpdateStatus | null>(null);
  const [restartError, setRestartError] = useState("");
  const [meta, setMeta] = useState<api.ItemMeta[]>([]);
  const [est, setEst] = useState<Record<string, api.Estimate>>({});
  const [links, setLinks] = useState<api.Links>({ links: [] });
  const [blockers, setBlockers] = useState<api.Blockers>({ blockers: [] });
  const [brief, setBrief] = useState<api.Brief | null>(null);
  const [talk, setTalk] = useState<Talk | null>(null);
  const [skipped, setSkipped] = useState<string[]>([]);
  // A new day: the talk of yesterday goes.
  useEffect(() => setTalk(null), [today]);
  const [goals, setGoals] = useState<api.Goals | null>(null);
  const [briefLoaded, setBriefLoaded] = useState("");
  const [briefState, setBriefState] = useState("idle");
  /** The item that you work on now, with its timer. */
  const [work, setWork] = useState<api.Running | null>(null);
  // The mode of the home box stays when you go to another page.
  const [homeMode, setHomeMode] = useState<"ask" | "task">("ask");
  const [reviewCards, setReviewCards] = useState<{ day: CardMode; week: CardMode }>({ day: "auto", week: "auto" });
  // A card that you closed comes back on a new day.
  useEffect(() => setReviewCards({ day: "auto", week: "auto" }), [today]);
  /** The plans of today and the next days, by day. null: the day has no plan. */
  const [plans, setPlans] = useState<Record<string, api.DayPlan | null>>({});
  /** The plans as they are now, also before the next render. putPlan() sets
   * it at once, so two quick changes both start from the newest plan. */
  const plansNow = useRef(plans);
  plansNow.current = plans;
  /** The last save of each day. A new save of a day waits for it, so the
   * saves of one day run in the order the user made them. */
  const saveQueue = useRef<Record<string, Promise<unknown>>>({});
  /** A count of the saves of each day. Only the reply of the newest save may change the screen. */
  const saveSeq = useRef<Record<string, number>>({});
  /** A count of the changes to the plan of each day on the screen. loadPlans() uses it. */
  const planEdits = useRef<Record<string, number>>({});
  const plan = plans[today] ?? null;
  // The first day of the loaded plans. Until the plans of this day load, a
  // change must not save: it would use the plans of the day before.
  const [planLoaded, setPlanLoaded] = useState("");
  /** The day that the Calendar page shows, and how it shows it. */
  const [calDay, setCalDay] = useState(today);
  const [calView, setCalViewState] = useState<"day" | "week">(() => (stored("aifred-cal-view") === "week" ? "week" : "day"));
  const [models, setModels] = useState<AppCtx["models"]>(() => ({
    main: null,
    plan: stored("aifred-model-plan") || null,
    // "" is a saved pick of Default. No saved pick at all is Haiku.
    item: ((m) => (m === null ? "haiku" : m || null))(stored("aifred-model-item")),
  }));

  /** False until the saved main model is loaded. Before that, a question
   * sends no model, so the backend uses the saved pick itself. */
  const mainModelKnown = useRef(false);
  const [main, setMain] = useState<ChatState>(EMPTY_CHAT);
  const [mainOpen, setMainOpen] = useState(false);
  /** The plan chat of each day, by day. Each day keeps its own messages and
   * its own running answer, so a change of day does not lose them. */
  const [planChats, setPlanChats] = useState<Record<string, ChatState>>({});
  const [planChatOpen, setPlanChatOpen] = useState(false);
  /** The first day of the plans on the screen. */
  const planDay = useRef("");
  /** The day of the plan chat on the screen. */
  const [chatDay, setChatDay] = useState("");
  /** The history load of each plan chat, by day. It never fails: a failed load is taken out, so the next open tries again. */
  const chatLoaded = useRef(new Map<string, Promise<void>>());
  /** The day of the plan answer that runs now. A ref, so two clicks before the next render cannot both start one. */
  const planRunning = useRef<string | null>(null);
  const planChat = planChats[chatDay] ?? EMPTY_CHAT;
  /** The day of the plan chat that is answering now, or undefined. Only one
   * plan chat runs at a time: two answers could change the same plans. */
  const planBusyDay = Object.keys(planChats).find((d) => planChats[d].busy);
  const [itemChats, setItemChats] = useState<Record<string, ChatState>>({});
  // A restart to update must wait for every running answer and the brief.
  const anyBusy = main.busy || planBusyDay !== undefined || Object.values(itemChats).some((c) => c.busy) || briefState === "writing";
  const [sel, setSel] = useState<string | null>(null);
  /** The id of the mission that is open on the right side. Only one of
   * `sel` and `selMission` is set: the side shows one item or one mission. */
  const [selMission, setSelMission] = useState<number | null>(null);
  /** Closes the item or the mission on the right side. */
  const clearSel = () => {
    setSel(null);
    setSelMission(null);
  };
  const [side, setSide] = useState<"chat" | "task">("chat");
  const [unread, setUnread] = useState(false);
  const [toastState, setToast] = useState<Toast | null>(null);
  const [closed, setClosed] = useState<Closed[]>([]);
  /** The ClickUp tasks that are closing now. A second click on one does nothing. */
  const closing = useRef(new Set<string>());
  // The email threads that are on their way to hidden or to the "waiting for"
  // list. A double click (or Hide and Track together) changes a thread once.
  const mailBusy = useRef(new Set<string>());
  const [sideW, setSideW] = useState<number | null>(() => {
    const w = Number(stored("aifred-side-w"));
    return w >= 300 ? w : null;
  });
  const [, setWinW] = useState(0);
  const appRef = useRef<HTMLDivElement>(null);
  const githubLoad = useRef(0);
  const clickupLoad = useRef(0);
  /** True after a load while ClickUp was connected. A disconnect then reloads the links and blockers. */
  const clickupWasOn = useRef(false);
  const briefTried = useRef(new Set<string>());
  const now = useRef({ sel, side, mission: selMission });
  now.current = { sel, side, mission: selMission };

  // ---------- Loading ----------

  const loadTasks = useCallback(() => {
    api
      .tasks()
      .then((t) => {
        setTasks(t);
        setTasksError("");
        setTasksLoaded(true);
      })
      .catch((e) => setTasksError(String(e)));
  }, []);
  const loadMeta = useCallback(() => {
    api.itemMeta().then(setMeta).catch(() => {});
    api.estimates().then(setEst).catch(() => {});
    api.itemLinks().then(setLinks).catch(() => {});
    api.itemBlockers().then(setBlockers).catch(() => {});
  }, []);

  /** Loads can overlap (reload, then a tick). Only the answer to the newest load may change the screen. */
  const loadGithub = useCallback(
    (force: boolean) => {
      const n = ++githubLoad.current;
      setGithubState("loading");
      api
        .githubItems(force)
        .then((list) => {
          if (n !== githubLoad.current) return;
          setGithub(list);
          setGithubState("ok");
          loadMeta();
        })
        .catch((e) => n === githubLoad.current && setGithubState(String(e)));
    },
    [loadMeta],
  );

  const calendarLoad = useRef(0);
  const loadCalendar = useCallback((force: boolean) => {
    const n = ++calendarLoad.current;
    setCalendarState("loading");
    api
      .calendarEvents(force)
      .then((list) => {
        if (n !== calendarLoad.current) return;
        // null: Google is not connected. Then there is no calendar, and no error.
        setCalendar(list ?? { events: [], fetched_at: 0 });
        setCalendarState(list ? "ok" : "off");
      })
      .catch((e) => n === calendarLoad.current && setCalendarState(String(e)));
  }, []);

  // A change on the screen (close, add, delete) adds 1 to an "edits"
  // counter. A load that started before the change can bring the list from
  // before it. Then the load does not show its answer: it asks again.
  const waitingLoad = useRef(0);
  const waitingEdits = useRef(0);
  const loadWaiting = useCallback(() => {
    const go = () => {
      const n = ++waitingLoad.current;
      const edits = waitingEdits.current;
      api
        .waitingItems()
        .then((l) => {
          if (n !== waitingLoad.current) return;
          if (edits !== waitingEdits.current) return go();
          setWaiting(l);
          setWaitingError("");
        })
        .catch((e) => n === waitingLoad.current && setWaitingError(String(e)));
    };
    go();
  }, []);

  const mailLoad = useRef(0);
  const mailEdits = useRef(0);
  /**
   * true or false: the "force" of a load. "saved": the saved list at any age, with no download.
   * "timer": a download that keeps the shown list on screen while it runs. Gmail can be slow.
   */
  const loadMail = useCallback(
    (how: boolean | "saved" | "timer") => {
      const go = (how: boolean | "saved" | "timer") => {
        const n = ++mailLoad.current;
        const edits = mailEdits.current;
        if (how === true || how === false) setMailState("loading");
        (how === "saved" ? api.gmailSaved() : api.gmailThreads(how !== false))
          .then((list) => {
            if (n !== mailLoad.current) return;
            // A hide or a track during the load: the saved list has the change.
            if (edits !== mailEdits.current) return go("saved");
            // null: Gmail is not connected. Then there are no emails, and no error.
            setMail(list ?? { reply: [], sent: [], fetched_at: 0 });
            setMailState(list ? "ok" : "off");
            loadMeta();
            // A download closes the "waiting for" items whose person answered.
            loadWaiting();
          })
          .catch((e) => n === mailLoad.current && setMailState(String(e)));
      };
      go(how);
    },
    [loadMeta, loadWaiting],
  );

  const loadClickup = useCallback(
    (force: boolean, connected: boolean) => {
      const n = ++clickupLoad.current;
      if (!connected) {
        setClickup({ items: [], fetched_at: 0 });
        setClickupState("off");
        // After a disconnect, the backend took away the links and blockers of
        // the ClickUp items. When ClickUp was not connected, nothing changed.
        if (clickupWasOn.current) loadMeta();
        clickupWasOn.current = false;
        return;
      }
      clickupWasOn.current = true;
      setClickupState("loading");
      api
        .clickupItems(force)
        .then((list) => {
          if (n !== clickupLoad.current) return;
          setClickup(list);
          setClickupState("ok");
          loadMeta();
        })
        .catch((e) => n === clickupLoad.current && setClickupState(String(e)));
    },
    [loadMeta],
  );

  const checkClickup = useCallback(
    (force: boolean) => {
      api
        .clickupTokenStatus()
        .then((t) => {
          setClickupToken(t);
          loadClickup(force, t.set);
        })
        .catch((e) => setClickupState(String(e)));
    },
    [loadClickup],
  );
  const clickupOn = clickupToken?.set ?? false;

  useEffect(() => {
    const failed = (what: string) => (e: unknown) => setLoadError(`Cannot load ${what}: ${e}`);
    api
      .history()
      .then((turns) => setMain((c) => ({ ...c, msgs: turnMsgs(turns) })))
      .catch(failed("the conversation"));
    api.info().then(setInfo).catch(failed("app info"));
    api.memoryFacts().then(setFacts).catch(failed("memory"));
    api.workRunning().then(setWork).catch(() => {});
    api
      .getSettings()
      .then((s) => setSettings({ ...api.DEFAULT_SETTINGS, ...s }))
      .catch((e) => {
        failed("settings")(e);
        setSettings(api.DEFAULT_SETTINGS);
      });
    api
      .chatModel()
      .then((m) => {
        mainModelKnown.current = true;
        setModels((all) => ({ ...all, main: m }));
      })
      .catch(failed("the chat model"));
    api.goals().then(setGoals).catch(failed("your goals"));
    loadTasks();
    loadMeta();
    loadGithub(false);
    loadCalendar(false);
    loadMail(false);
    loadWaiting();
    checkClickup(false);
  }, [loadTasks, loadMeta, loadGithub, loadCalendar, loadMail, loadWaiting, checkClickup]);

  /** Loads the plans of today and the next days. A day that changed on the
   * screen after the load started keeps the newer plan of the screen. */
  const loadPlans = useCallback((tries = 3) => {
    const from = today;
    const seen = { ...planEdits.current };
    api
      .dayPlans(from, addDays(from, WEEK - 1))
      .then((list) => {
        if (planDay.current !== from) return;
        const got: Record<string, api.DayPlan | null> = {};
        for (let n = 0; n < WEEK; n++) got[addDays(from, n)] = null;
        for (const p of list) got[p.day] = p;
        setPlans((old) => {
          const m = { ...got };
          for (const day of Object.keys(m)) if (planEdits.current[day] !== seen[day] && day in old) m[day] = old[day];
          return m;
        });
        setPlanLoaded(from);
      })
      .catch(() => {
        // Try again soon. Until the plans load, the plan changes stay off.
        if (tries > 1) setTimeout(() => planDay.current === from && loadPlans(tries - 1), 3000);
      });
  }, [today]);

  /** Shows the plan chat of `day`, with its old messages. The promise ends when the old messages are in. */
  const openPlanChat = useCallback((day: string): Promise<void> => {
    setChatDay(day);
    const loading = chatLoaded.current.get(day);
    if (loading) return loading;
    const load = api
      .sideHistory(`plan:${day}`)
      .then((t) => setPlanChats((all) => ({ ...all, [day]: { ...(all[day] ?? EMPTY_CHAT), msgs: [...turnMsgs(t), ...(all[day]?.msgs ?? [])] } })))
      .catch(() => {
        chatLoaded.current.delete(day);
      });
    chatLoaded.current.set(day, load);
    return load;
  }, []);

  /** Shows `day` on the Calendar page, with its plan chat. */
  const pickDay = useCallback(
    (day: string) => {
      setCalDay(day);
      openPlanChat(day);
    },
    [openPlanChat],
  );

  // The plans, the brief, and the plan chat belong to one day. A new day loads them again.
  useEffect(() => {
    let live = true;
    planDay.current = today;
    setPlans({});
    loadPlans();
    setCalDay(today);
    openPlanChat(today);
    api
      .brief(today)
      .then((b) => {
        if (!live) return;
        setBrief(b);
        setBriefLoaded(today);
      })
      .catch(() => live && setBriefLoaded(today));
    return () => {
      live = false;
    };
  }, [today, loadPlans, openPlanChat]);

  // A repeating task: AIfred adds the new task by itself. Show it at once.
  useTasksChanged(loadTasks);

  /** Reads the brief of `day` again, when it is still on the screen. */
  const reloadBrief = useCallback((day: string | undefined) => {
    if (day) api.brief(day).then((b) => b && setBrief((old) => (old?.day === day ? b : old))).catch(() => {});
  }, []);

  /** Reads today's brief again and shows it. Only a newer brief of the same
   * day on the screen stays. The brief can change with no new text: for
   * example its missions or its note about AIfred's moves. */
  const loadTodayBrief = useCallback(() => {
    api
      .brief(today)
      .then((b) => b && setBrief((old) => (old && old.day === b.day && old.written > b.written ? old : b)))
      .catch(() => {});
  }, [today]);

  // AIfred's sort moved new items into subtasks in the background. Show the
  // new trees, and the brief with its note about the moves, at once.
  const afterSort = useCallback(() => {
    api.itemLinks().then(setLinks).catch(() => {});
    // The sort also takes away the blockers of gone items (links.rs drop_gone_in).
    api.itemBlockers().then(setBlockers).catch(() => {});
    loadTodayBrief();
  }, [loadTodayBrief]);
  useItemsSorted(afterSort);

  // AIfred's jobs can write the brief in the background (for example when you
  // finish a task). Check for a newer brief each minute.
  useEffect(() => {
    if (briefState === "writing") return;
    const timer = setInterval(() => {
      api
        .brief(today)
        .then((b) => b && setBrief((old) => (old && old.written >= b.written && old.day === b.day ? old : b)))
        .catch(() => {});
    }, 60_000);
    return () => clearInterval(timer);
  }, [today, briefState]);

  // Reload the GitHub and ClickUp lists on the timer from the settings. 0 means never.
  const refreshMinutes = settings?.refresh_minutes ?? 0;
  useEffect(() => {
    if (refreshMinutes <= 0) return;
    const timer = setInterval(() => {
      loadGithub(true);
      loadClickup(true, clickupOn);
      loadCalendar(true);
      loadMail("timer");
    }, refreshMinutes * 60_000);
    return () => clearInterval(timer);
  }, [refreshMinutes, loadGithub, loadClickup, loadCalendar, loadMail, clickupOn]);

  // The first start downloads the meaning search model. Check again until it is ready.
  const meaningStarting = info?.meaning_search === "starting";
  useEffect(() => {
    if (!meaningStarting) return;
    const timer = setInterval(() => api.info().then(setInfo).catch(() => {}), 10_000);
    return () => clearInterval(timer);
  }, [meaningStarting]);

  // The app checks for updates by itself (main.rs). Read its status each minute.
  useEffect(() => {
    const load = () => api.updateStatus().then(setUpdate).catch(() => {});
    load();
    const timer = setInterval(load, 60_000);
    return () => clearInterval(timer);
  }, []);

  /** Writes today's brief. `release` true: the brief stops holding its
   * missions for the questions (brief.rs), also when the server still sees
   * a question that this window does not show. */
  const writeBriefWith = useCallback((release: boolean) => {
    const day = today;
    briefTried.current.add(day);
    setBriefState("writing");
    api
      .writeBrief(day, release)
      .then((b) => {
        if (b.day === dayOf(new Date())) setBrief(b);
        setBriefState("idle");
        api.estimates().then(setEst).catch(() => {});
      })
      .catch((e) => setBriefState(String(e)));
  }, [today]);
  const writeBrief = useCallback(() => writeBriefWith(false), [writeBriefWith]);

  // The load state of each source of items, by the start of the item key ("github:...").
  const sourceStates = useMemo<Record<string, ListState>>(
    () => ({ github: githubState, clickup: clickupState, gmail: mailState }),
    [githubState, clickupState, mailState],
  );
  const sourcesLoading = Object.values(sourceStates).includes("loading");

  // No question is left, and you answered one of them on the home page
  // after the brief read the items: write the brief again, so that its text
  // and missions use the new notes. The time of the answer counts, not the
  // brief on the screen: a newer brief can load between two answers.
  // A brief that held its missions for the questions also writes again
  // after skips only, but one time for each day: the server can still see
  // a question that the card does not show (for example when a list did
  // not load). So that write also asks the server to stop the hold
  // (brief::release), and the missions come. One time for each
  // brief, so a failed write does not repeat. It waits while the last talk
  // stays on the screen: you can still be reading it.
  const heldTried = useRef(new Set<number>());
  // The days whose held brief was written again with no new answer.
  const heldDays = useRef(new Set<string>());
  // When you last sent an answer on the question card, in Unix seconds (as
  // brief.written). 0: no answer since the last rewrite.
  const lastAnswer = useRef(0);
  const noteAnswer = useCallback(() => {
    lastAnswer.current = Math.floor(Date.now() / 1000);
  }, []);

  // The brief writes itself once a day, after the lists are loaded, when the setting is on.
  // It does not wait for Gmail: the brief gets the emails itself, and Gmail can be slow.
  const listsReady = githubState !== "loading" && clickupState !== "loading";
  useEffect(() => {
    if (!settings?.auto_brief || briefLoaded !== today || brief || !listsReady || briefTried.current.has(today)) return;
    writeBrief();
  }, [settings?.auto_brief, briefLoaded, today, brief, listsReady, writeBrief]);

  // ---------- Items ----------

  const goalText = useCallback((id: number | undefined) => goals?.goals.find((g) => g.id === id)?.text, [goals]);
  const items = useMemo(() => {
    // The goal of each item that the brief ranked. A goal that the user removed since then is left out.
    const goalOf: Record<string, string> = {};
    for (const [key, id] of Object.entries(brief?.goal ?? {})) {
      const t = goalText(id);
      if (t) goalOf[key] = t;
    }
    return buildItems(tasks, github.items, clickup.items, meta, est, brief?.why ?? {}, goalOf, mail.reply, brief?.next, brief?.draft);
  }, [tasks, github.items, clickup.items, meta, est, brief, goalText, mail.reply]);

  const byKey = useMemo(() => new Map(items.map((i) => [i.key, i])), [items]);

  // The rewrite after the questions (see heldTried above). It uses the same
  // rule as the question card (openQuestions), so a question that the card
  // does not show cannot hold the missions for the whole day. It waits for
  // the GitHub and ClickUp lists: while they load, the card shows no question.
  useEffect(() => {
    if (!brief || brief.day !== today || briefState === "writing" || talk) return;
    if (githubState === "loading" || clickupState === "loading") return;
    if (openQuestions(brief, byKey, today, skipped, githubState, clickupState).length > 0) return;
    // The same second counts: the brief can have read the items before the answer.
    const answered = lastAnswer.current > 0 && lastAnswer.current >= brief.written;
    if (!answered && (!brief.held || heldDays.current.has(brief.day))) return;
    if (heldTried.current.has(brief.written)) return;
    heldTried.current.add(brief.written);
    if (!answered) heldDays.current.add(brief.day);
    // The rewrite uses the answers up to now.
    lastAnswer.current = 0;
    // The card shows no question now. So the brief must stop holding: the
    // server can still see a question that the card does not show, for
    // example a ClickUp task that is not in the loaded list. Else the
    // missions wait for the whole day.
    writeBriefWith(true);
  }, [brief, today, briefState, writeBriefWith, talk, byKey, skipped, githubState, clickupState]);
  const cfg = settings ?? api.DEFAULT_SETTINGS;
  /** The open plan items and the schedule of today and each next day. */
  // The events of each day do not change when an item changes: keep them apart.
  const weekEvents = useMemo(() => {
    const out: Record<string, DayEvents> = {};
    for (let n = 0; n < WEEK; n++) {
      const day = addDays(today, n);
      out[day] = eventsOn(calendar.events, day);
    }
    return out;
  }, [today, calendar.events]);
  const week = useMemo(() => {
    const out: Record<string, { items: Item[]; sched: Schedule; events: DayEvents }> = {};
    for (const [day, events] of Object.entries(weekEvents)) {
      const p = plans[day]?.items ?? [];
      const dayItems = p.map((x) => byKey.get(x.key)).filter((i): i is Item => !!i);
      const pins: Record<string, number> = {};
      for (const x of p) if (x.at !== undefined) pins[x.key] = x.at;
      out[day] = { items: dayItems, sched: schedule(dayItems, cfg, events.busy, pins), events };
    }
    return out;
  }, [weekEvents, plans, byKey, cfg]);
  const planItems = week[today].items;
  const sched = week[today].sched;

  // An item that left the list (closed in ClickUp, deleted) closes its panel.
  useEffect(() => {
    if (sel && !byKey.has(sel) && !sourcesLoading) setSel(null);
  }, [sel, byKey, sourcesLoading]);
  // A mission that left today's list (done, "Not today", a new brief) closes its panel.
  useEffect(() => {
    if (selMission !== null && brief && !brief.missions?.some((m) => m.id === selMission)) setSelMission(null);
  }, [selMission, brief]);

  // ---------- Toast ----------

  const toastTimer = useRef<number | undefined>(undefined);
  const toast = useCallback((text: string, undo?: () => void, error?: boolean) => {
    setToast({ text, undo, error });
    clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), error ? 6000 : 4000);
  }, []);

  /** Runs a change. A failure shows its reason. Returns true when it worked. */
  const run = useCallback(
    async <T,>(p: Promise<T>, after?: (v: T) => void): Promise<boolean> => {
      try {
        const v = await p;
        after?.(v);
        return true;
      } catch (e) {
        toast(String(e), undefined, true);
        return false;
      }
    },
    [toast],
  );

  // Goal changes run one at a time, in the order the user made them. A
  // click on ✕ first ends the edit in the box (blur), so an edit and a
  // remove start together. Without the queue, the edit's answer can come
  // last and show the removed goal again.
  const goalQueue = useRef<Promise<unknown>>(Promise.resolve());
  const changeGoals = useCallback(
    (change: () => Promise<api.Goals>) => {
      const p = goalQueue.current.catch(() => undefined).then(change);
      goalQueue.current = p;
      return run(p, setGoals);
    },
    [run],
  );

  // ---------- Actions on items ----------

  const putTask = (t: api.Task) => setTasks((all) => all.map((x) => (x.id === t.id ? t : x)));

  // ---------- Work time ----------

  /** Ends the work on a finished item, and keeps its real time next to the
   * time guess (worktime.rs). The timer leaves the screen only when the
   * save worked, so a failed save never hides a timer that still runs.
   * Throws when the save fails. */
  const endWork = useCallback(async (key: string) => {
    const d = await api.finishWork(key);
    setWork((w) => (w?.key === key ? null : w));
    return d;
  }, []);

  /** The toast for a finished item, with its real time `d` that is already
   * kept. `text` null: no toast when the item has no work time. `undo`
   * opens the item again. With `forgetsTime`, the undo itself makes the
   * backend forget the real time, so the screen does not ask for it. */
  const finishedToast = useCallback(
    (key: string, text: string | null, d: api.WorkDone | null, undo?: () => void, forgetsTime = false) => {
      if (text === null && !d) return;
      const msg = text === null ? `Finished: ${d!.title}.${tookText(d)}` : d ? `${text}.${tookText(d)}` : text;
      toast(
        msg,
        undo &&
          (() => {
            if (d && !forgetsTime) api.unfinishWork(key).catch(() => {});
            undo();
          }),
      );
    },
    [toast],
  );

  /** Stops the work on a finished item, keeps its real time, and shows
   * the toast of finishedToast(). */
  const afterFinish = useCallback(
    async (key: string, text: string | null, undo?: () => void) => {
      let d: api.WorkDone | null;
      try {
        d = await endWork(key);
      } catch (e) {
        // The timer stays on screen, so you can click Done again.
        toast(`${text ? `${text}. ` : ""}The work time is not saved: ${e}`, undo, true);
        return;
      }
      finishedToast(key, text, d, undo);
    },
    [toast, endWork, finishedToast],
  );

  /**
   * Hides an email in AIfred, with Undo. The email in Gmail does not change.
   * `list`: the list that the email leaves, "reply" or "sent". `workKey`:
   * the email is finished, so its work ends too. Its real time goes into the
   * same toast, and Undo opens both again: a second toast would replace
   * the Undo of the hide.
   */
  const hideMail = useCallback(
    async (m: api.Mail, list: "reply" | "sent", text: string, workKey?: string, keepUndo?: (undo: () => void) => void): Promise<boolean> => {
      if (mailBusy.current.has(m.id)) return false;
      mailBusy.current.add(m.id);
      try {
        const ok = await run(api.hideEmail(m.id, true), () => {
          mailEdits.current++;
          setMail((l) => ({ ...l, [list]: l[list].filter((x) => x.id !== m.id) }));
        });
        if (!ok) return false;
        let d: api.WorkDone | null = null;
        let note = "";
        if (workKey) {
          try {
            d = await endWork(workKey);
          } catch (e) {
            note = ` The work time is not saved: ${e}`;
          }
        }
        const undo = () =>
          run(api.hideEmail(m.id, false), () => {
            mailEdits.current++;
            setMail((l) => ({ ...l, [list]: putBack(l[list], m) }));
            if (d && workKey) api.unfinishWork(workKey).catch(() => {});
          });
        if (keepUndo) keepUndo(undo);
        else toast(`${text}${tookText(d)}${note}`, undo);
        return true;
      } finally {
        mailBusy.current.delete(m.id);
      }
    },
    [run, toast, endWork],
  );

  const startWork = useCallback((key: string) => void run(api.startWork(key), setWork), [run]);
  const stopWork = useCallback(() => void run(api.stopWork(), () => setWork(null)), [run]);

  /** Closes or opens `i`. Each close shows a toast with an undo. With
   * `keepUndo`, a task or an email gives its undo to the caller and shows
   * no toast: finishMission() puts all undos in one toast. A ClickUp close
   * keeps its own undo in the list of closed ClickUp tasks, and with
   * `keepUndo` it shows no toast either. */
  const toggleDone = useCallback(
    (i: Item, keepUndo?: (undo: () => void) => void) => {
      if (i.task) {
        const done = !i.done;
        // The backend ends the work with the task and keeps its real time,
        // or opens it again (tasks.rs). So the screen does not save the
        // time a second time. The timer goes in the same draw as the done
        // task: the check for an item that closed elsewhere (below) must
        // not finish it a second time. A failed change keeps the timer,
        // because the work still runs.
        return run(api.setTaskDone(i.task.id, done), ({ task: t, work: d }) => {
          if (done) setWork((w) => (w?.key === i.key ? null : w));
          putTask(t);
          // The undo opens the task again. Then the backend forgets its real time.
          const undo = () => run(api.setTaskDone(t.id, false), (r) => putTask(r.task));
          if (done && keepUndo) keepUndo(undo);
          else if (done) finishedToast(i.key, `Finished: ${i.title}`, d, undo, true);
        });
      } else if (i.mail) {
        return hideMail(i.mail, "reply", `Hidden: ${i.mail.subject}. It comes back when a new message comes.`, i.key, keepUndo);
      } else if (i.cu) {
        const c = i.cu;
        if (closing.current.has(c.id)) return Promise.resolve(false);
        closing.current.add(c.id);
        // This changes the real task in ClickUp, so there is an undo.
        return run(api.clickupMarkDone(c.id), (before) => {
          setClickup((l) => ({ ...l, items: l.items.filter((x) => x.id !== c.id) }));
          setClosed((l) => [...l.filter((x) => x.id !== c.id), { id: c.id, title: c.title, before }]);
          // In a mission, the work ends with no toast of its own: it must
          // not replace the toast of the mission and its Undo. A failed save
          // keeps the timer on the screen, so you can click Done again.
          if (keepUndo) void endWork(i.key).catch(() => {});
          else void afterFinish(i.key, null);
        }).finally(() => closing.current.delete(c.id));
      }
      return Promise.resolve(false);
    },
    [run, hideMail, afterFinish, finishedToast, endWork],
  );

  /** The "Done" button of the work timer. AIfred only reads GitHub, so a
   * GitHub item keeps its place in the list until it closes there. An item
   * that left the list (for example, closed in ClickUp) only ends its work. */
  const finishItem = useCallback(
    (key: string) => {
      const i = byKey.get(key);
      if (!i) void afterFinish(key, null);
      else if (i.gh) void afterFinish(i.key, `Finished: ${i.title}`, () => {});
      else toggleDone(i);
    },
    [byKey, afterFinish, toggleDone],
  );

  const undoClose = useCallback(
    (c: Closed) =>
      run(api.clickupUndoDone(c.id, c.before), () => {
        api.unfinishWork(`clickup:${c.id}`).catch(() => {});
        setClosed((l) => l.filter((x) => x.id !== c.id));
        loadClickup(true, true);
      }),
    [run, loadClickup],
  );

  // ---------- The "waiting for" list ----------

  const putWaiting = (w: api.Waiting) => {
    waitingEdits.current++;
    setWaiting((all) => (all.some((x) => x.id === w.id) ? all.map((x) => (x.id === w.id ? w : x)) : [...all, w]));
  };
  const dropWaiting = (id: number) => {
    waitingEdits.current++;
    setWaiting((all) => all.filter((x) => x.id !== id));
  };

  /** Puts an email that you sent on the "waiting for" list. */
  const trackEmail = useCallback(
    (m: api.Mail, followUp?: string): Promise<boolean> => {
      if (mailBusy.current.has(m.id)) return Promise.resolve(false);
      mailBusy.current.add(m.id);
      return run(api.addWaiting({ thread: m.id, follow_up: followUp }), (w) => {
        putWaiting(w);
        mailEdits.current++;
        setMail((l) => ({ ...l, sent: l.sent.filter((x) => x.id !== m.id) }));
        toast(`Waiting for ${w.who}: ${w.what}`, () =>
          run(api.deleteWaiting(w.id), () => {
            dropWaiting(w.id);
            mailEdits.current++;
            setMail((l) => ({ ...l, sent: putBack(l.sent, m) }));
          }),
        );
      }).finally(() => mailBusy.current.delete(m.id));
    },
    [run, toast],
  );

  /** Hides an email that you sent from the suggestions: you wait for nothing. */
  const hideSent = useCallback((m: api.Mail) => hideMail(m, "sent", `Hidden: ${m.subject}`), [hideMail]);

  const addWaiting = useCallback(
    (item: api.NewWaiting) =>
      run(api.addWaiting(item), (w) => {
        putWaiting(w);
        // An item from an email takes it off the suggestions.
        if (item.thread) {
          mailEdits.current++;
          setMail((l) => ({ ...l, sent: l.sent.filter((x) => x.id !== item.thread) }));
        }
      }),
    [run],
  );

  const changeWaiting = useCallback(
    (id: number, changes: api.WaitingChanges) =>
      run(api.updateWaiting(id, changes), (w) => {
        putWaiting(w);
        if (changes.done) toast(`Closed: ${w.what}`, () => run(api.updateWaiting(w.id, { done: false }), putWaiting));
      }),
    [run, toast],
  );

  const removeWaiting = useCallback(
    (w: api.Waiting) =>
      run(api.deleteWaiting(w.id), () => {
        dropWaiting(w.id);
        // An email item goes back to the suggestions. The saved list has
        // it: no download from Gmail.
        if (w.thread) {
          mailEdits.current++;
          loadMail("saved");
        }
      }),
    [run, loadMail],
  );

  const addTask = useCallback(
    (fields: api.TaskChanges & { title: string }) =>
      run(api.createTask(fields), (t) => {
        setTasks((all) => [...all, t]);
        toast(`Added: ${t.title}`, () => run(api.deleteTask(t.id), () => setTasks((all) => all.filter((x) => x.id !== t.id))));
        loadMeta();
      }),
    [run, toast, loadMeta],
  );

  const editTask = useCallback((id: string, changes: api.TaskChanges) => run(api.editTask(id, changes), putTask), [run]);

  const addSteps = useCallback(
    (key: string, steps: string[]) =>
      run(api.addSteps(key, steps), (a) => {
        const t = a.task;
        const loadLinks = () => {
          api.itemLinks().then(setLinks).catch(() => {});
          api.itemBlockers().then(setBlockers).catch(() => {});
        };
        if (a.created) {
          setTasks((all) => [...all, t]);
          loadLinks();
          toast(`Made the task "${t.title}" with ${a.added} step${a.added === 1 ? "" : "s"}, under this item`, () =>
            run(api.deleteTask(t.id), () => {
              setTasks((all) => all.filter((x) => x.id !== t.id));
              loadLinks();
            }),
          );
          return;
        }
        putTask(t);
        if (!a.added) {
          toast(`"${t.title}" has these steps already`);
          return;
        }
        const added = t.checklist.slice(t.checklist.length - a.added).map((c) => c.text);
        // Undo takes out the added steps that are not done, in one save. Other changes since then stay.
        toast(`Added ${a.added} step${a.added === 1 ? "" : "s"} to "${t.title}"`, () => run(api.removeTaskSteps(t.id, added), putTask));
      }),
    [run, toast],
  );

  const tickStep = useCallback(
    (id: string, index: number) => {
      const t = tasks.find((x) => x.id === id);
      const c = t?.checklist[index];
      if (c) run(api.setTaskStep(id, index, c.text, !c.done), putTask);
    },
    [tasks, run],
  );

  const deleteTask = useCallback(
    (i: Item) => {
      if (!i.task) return;
      const t = i.task;
      run(api.deleteTask(t.id), () => {
        // The backend deletes the task's work sessions too (tasks.rs).
        setWork((w) => (w?.key === i.key ? null : w));
        setTasks((all) => all.filter((x) => x.id !== t.id));
        if (now.current.sel === i.key) setSel(null);
        // The backend takes the task out of its group and its blockers too (tasks.rs).
        api.itemLinks().then(setLinks).catch(() => {});
        api.itemBlockers().then(setBlockers).catch(() => {});
        toast(`Deleted: ${t.title}`);
      });
    },
    [run, toast],
  );

  // ---------- The plan ----------

  // Only a plan of the day on screen may change the screen. An answer
  // that comes back after midnight belongs to the old day.
  const putPlan = useCallback((day: string, p: api.DayPlan | null) => {
    const from = planDay.current;
    if (day < from || day > addDays(from, WEEK - 1)) return;
    planEdits.current[day] = (planEdits.current[day] ?? 0) + 1;
    plansNow.current = { ...plansNow.current, [day]: p };
    setPlans((m) => ({ ...m, [day]: p }));
  }, []);

  /** True when the plans are loaded, so a change can start from them. Else
   * it shows a message and starts a new load. */
  const plansReady = useCallback(
    (day: string = today) => {
      if (planLoaded === today && day in plansNow.current) return true;
      loadPlans(1);
      toast(`The plan for ${dayName(day, today)} is still loading. Try again in a moment.`, undefined, true);
      return false;
    },
    [planLoaded, today, loadPlans, toast],
  );

  /** Sends one save of the plan of `day` after the saves before it. `put`:
   * shows the saved plan, but only when no newer save of the day started. */
  const queueSave = useCallback(
    <T,>(day: string, send: () => Promise<T>, put?: (v: T) => void): Promise<T> => {
      const seq = (saveSeq.current[day] ?? 0) + 1;
      saveSeq.current[day] = seq;
      const done = (saveQueue.current[day] ?? Promise.resolve()).then(send);
      saveQueue.current[day] = done.catch(() => {});
      return done.then((v) => {
        if (put && saveSeq.current[day] === seq) put(v);
        return v;
      });
    },
    [],
  );

  /** True when the list of the item is loaded, so an item that is not in it is gone. */
  const listLoaded = useCallback(
    (key: string) => {
      const source = key.slice(0, key.indexOf(":"));
      if (source === "task") return tasksLoaded;
      const state = sourceStates[source];
      return state === undefined || state === "ok" || state === "off";
    },
    [tasksLoaded, sourceStates],
  );

  /** "today's plan", or "the plan for Tue 10 Nov". */
  const planName = useCallback((day: string) => (day === today ? "today's plan" : `the plan for ${shortDay(day, today)}`), [today]);

  /**
   * Saves the plan of `day` that `change` makes from its old items. `text`
   * says what changed, with an undo.
   */
  const writePlan = useCallback(
    (day: string, change: (old: api.PlanItem[]) => api.PlanItem[], text?: string) => {
      if (!plansReady(day)) return;
      const before = plansNow.current[day] ?? null;
      const next = change(before?.items ?? []);
      // A change by hand makes a plan from a review your own plan.
      const mode = before?.mode === "ai" ? "ai" : "self";
      putPlan(day, { day, mode, items: next, updated: Date.now() / 1000 });
      const seq = (saveSeq.current[day] ?? 0) + 1;
      queueSave(day, () => api.setDayPlan(day, mode, next), (p) => putPlan(day, p))
        .then(() => {
          if (text) {
            toast(text, () => {
              putPlan(day, before);
              if (before) queueSave(day, () => api.setDayPlan(day, before.mode, before.items), (p) => putPlan(day, p)).catch(() => {});
              else queueSave(day, () => api.clearDayPlan(day)).catch(() => {});
            });
          }
        })
        .catch((e) => {
          // A newer change of the day is on the screen: it stays.
          if (saveSeq.current[day] === seq) putPlan(day, before);
          toast(`The plan is not saved: ${e}`, undefined, true);
        });
    },
    [plansReady, queueSave, toast, putPlan],
  );

  /**
   * The plan items for `keys` (the items on screen) in this order. A plan
   * item that is not on screen because its list did not load yet keeps its
   * place, so the save does not delete it. An item keeps the reason, the
   * goal, and the fixed time that it had.
   */
  const withKeys = useCallback(
    (items: api.PlanItem[], keys: string[]): api.PlanItem[] => {
      const old = items.map((p) => p.key);
      const shown = new Set(keys);
      const hidden = new Set(old.filter((k) => !shown.has(k) && !byKey.has(k) && !listLoaded(k)));
      const merged: string[] = [];
      let n = 0;
      for (const k of old) {
        if (hidden.has(k)) merged.push(k);
        else if (n < keys.length) merged.push(keys[n++]);
      }
      merged.push(...keys.slice(n));
      const byOld = new Map(items.map((p) => [p.key, p]));
      return merged.map((key) => byOld.get(key) ?? { key });
    },
    [byKey, listLoaded],
  );

  /** Saves a new order of the plan of `day` (today when not given). */
  const savePlan = useCallback(
    (keys: string[], text?: string, day: string = today) => writePlan(day, (old) => withKeys(old, keys), text),
    [writePlan, withKeys, today],
  );

  /** True, with a message, when the task `key` starts after `day`: it cannot go on that day. */
  const startsAfter = useCallback(
    (key: string, day: string) => {
      const i = byKey.get(key);
      if (!i?.start || i.start <= day) return false;
      toast(`${i.title} starts on ${shortDay(i.start, today)}, so it cannot go on ${shortDay(day, today)}.`, undefined, true);
      return true;
    },
    [byKey, toast, today],
  );
  const keysOf = useCallback((day: string) => (week[day]?.items ?? []).map((i) => i.key), [week]);
  const addToPlan = useCallback(
    (key: string, opts: { index?: number; day?: string; at?: number } = {}) => {
      const day = opts.day ?? today;
      if (startsAfter(key, day)) return;
      const keys = keysOf(day).filter((k) => k !== key);
      keys.splice(opts.index ?? keys.length, 0, key);
      const name = planName(day);
      writePlan(
        day,
        (old) => withKeys(old, keys).map((p) => (p.key === key && opts.at !== undefined ? { ...p, at: opts.at } : p)),
        `Added to ${name}${opts.at !== undefined ? ` at ${fmtTime(opts.at)}` : ""}: ${byKey.get(key)?.title ?? ""}`,
      );
    },
    [today, keysOf, planName, writePlan, withKeys, byKey, startsAfter],
  );
  const removeFromPlan = useCallback(
    (key: string, day: string = today) => savePlan(keysOf(day).filter((k) => k !== key), `Taken out of the plan: ${byKey.get(key)?.title ?? ""}`, day),
    [today, keysOf, savePlan, byKey],
  );
  /** Fixes the start time of a plan item. null: AIfred places it in the next free time. */
  const setPlanTime = useCallback(
    (key: string, at: number | null, day: string) => {
      const title = byKey.get(key)?.title ?? "";
      writePlan(
        day,
        (old) =>
          old.map((p) => {
            if (p.key !== key) return p;
            const { at: _old, ...rest } = p;
            return at === null ? rest : { ...rest, at };
          }),
        at === null ? `No fixed time: ${title}` : `Moved to ${fmtTime(at)}: ${title}`,
      );
    },
    [byKey, writePlan],
  );
  /**
   * Moves a plan item from the plan of `from` to the plan of `to`, in one
   * change. The message `text` has an undo that puts the item back at its
   * old place and time, and that also runs `undoMore`. Resolves true when
   * the move is saved.
   */
  const movePlan = useCallback(
    (key: string, from: string, to: string, at: number | null, text: string, undoMore?: () => void) => {
      const old = plansNow.current[from]?.items ?? [];
      const n = old.findIndex((p) => p.key === key);
      // The plan on the screen can be older than the saved plan. Then the
      // undo puts the item at the end, with no fixed time.
      const index = n < 0 ? null : n;
      const oldAt = n < 0 ? null : (old[n].at ?? null);
      return run(api.movePlanItem(from, to, key, at, null), ([a, b]) => {
        putPlan(from, a);
        putPlan(to, b);
        toast(text, () => {
          undoMore?.();
          run(api.movePlanItem(to, from, key, oldAt, index), ([b2, a2]) => {
            putPlan(to, b2);
            putPlan(from, a2);
          });
        });
      });
    },
    [run, putPlan, toast],
  );

  /** Moves a plan item to the plan of another day, in one change. */
  const moveToDay = useCallback(
    (key: string, from: string, to: string, at: number | null) => {
      if (!plansReady(from)) return;
      if (startsAfter(key, to)) return;
      movePlan(key, from, to, at, `Moved to ${planName(to)}: ${byKey.get(key)?.title ?? ""}`);
    },
    [plansReady, byKey, movePlan, planName, startsAfter],
  );

  const tomorrow = useCallback(
    (i: Item) => {
      const t = i.task;
      if (!t) return;
      // Without today's plan, a planned task can stay in it with a new due day.
      if (!plansReady()) return;
      const next = addDays(today, 1);
      const before = { due: t.due ?? "", start: t.start ?? "" };
      const changes: api.TaskChanges = { due: next, ...(t.start && t.start > next ? { start: "" } : {}) };
      // A planned task moves to the plan of tomorrow too, in one change.
      const planned = (plansNow.current[today]?.items ?? []).some((p) => p.key === i.key);
      run(api.editTask(t.id, changes), (nt) => {
        putTask(nt);
        const undoTask = () => run(api.editTask(t.id, before), putTask);
        if (!planned) {
          toast(`Moved to tomorrow: ${t.title}`, undoTask);
          return;
        }
        movePlan(i.key, today, next, null, `Moved to tomorrow: ${t.title}`, undoTask).then((moved) => {
          // The task stays in today's plan: it must stay due on its old day too.
          // The error message of the move stays on the screen.
          if (!moved) api.editTask(t.id, before).then(putTask).catch(() => {});
        });
      });
    },
    [plansReady, today, run, toast, movePlan],
  );

  const setMinutes = useCallback((key: string, minutes: number) => {
    setEst((e) => ({ ...e, [key]: { minutes, by: "you" } }));
    api.setEstimate(key, minutes).catch(() => api.estimates().then(setEst).catch(() => {}));
  }, []);

  // ---------- Chats ----------

  const reloadAfterAnswer = useCallback(() => {
    // AIfred can add or finish tasks, add facts, and change the plan while it answers.
    loadTasks();
    loadMeta();
    api.memoryFacts().then(setFacts).catch(() => {});
    // In the goal queue: a reload that ends after a later goal change must
    // not show the old goals again.
    const goals = goalQueue.current.catch(() => undefined).then(() => api.goals());
    goalQueue.current = goals;
    goals.then(setGoals).catch(() => {});
    loadPlans();
    // AIfred can add items to a mission or take them out (missions.rs).
    loadTodayBrief();
    // AIfred can put an item under another item (move_item), and say that
    // an item waits for another item (set_blocker).
    api.itemLinks().then(setLinks).catch(() => {});
    api.itemBlockers().then(setBlockers).catch(() => {});
  }, [loadTasks, loadMeta, loadPlans, loadTodayBrief]);

  /** Runs one question in a chat and puts the messages in `set`. */
  async function chat(
    set: (f: (c: ChatState) => ChatState) => void,
    busy: boolean,
    text: string,
    files: api.FileRef[],
    ask: (progress: (e: api.ProgressEvent) => void) => Promise<api.Answer>,
  ): Promise<boolean> {
    if (busy || !text.trim()) return false;
    set((c) => ({ busy: true, progress: [], msgs: [...c.msgs, { role: "user", text: text.trim(), files }] }));
    // False after an error: the box that sent the text puts it back, so
    // you can send it again.
    let ok = true;
    try {
      const a = await ask((e) => set((c) => ({ ...c, progress: [...c.progress, api.describe(e)] })));
      set((c) => ({ ...c, msgs: [...c.msgs, { role: "assistant", text: a.text, info: a.info, facts: a.memory }] }));
    } catch (e) {
      ok = false;
      set((c) => ({ ...c, msgs: [...c.msgs, { role: "error", text: String(e) }] }));
    } finally {
      set((c) => ({ ...c, busy: false, progress: [] }));
      reloadAfterAnswer();
    }
    if ((now.current.sel || now.current.mission !== null) && now.current.side === "task") setUnread(true);
    return ok;
  }

  const sendMain = (text: string, files: api.FileRef[]) => {
    setMainOpen(true);
    setSide("chat");
    if (page !== "home") setPage("home");
    text = withFiles(text, files);
    const model = mainModelKnown.current ? pickArg(models.main) : undefined;
    return chat(setMain, main.busy, text, files, (p) => api.ask(text.trim(), p, { model, files: files.map((f) => f.id) }));
  };

  const sendPlan = (text: string, files: api.FileRef[] = [], day: string = calDay) => {
    // Check first: a send that cannot run must not change the chat on the screen.
    const busyDay = planRunning.current ?? planBusyDay;
    if (busyDay !== undefined) {
      toast(`AIfred is still working on ${planName(busyDay)}. Try again when it is done.`, undefined, true);
      return Promise.resolve(false);
    }
    // Files with no text send "Look at these files.".
    text = withFiles(text, files);
    if (!text.trim()) return Promise.resolve(false);
    setSide("chat");
    const history = openPlanChat(day);
    planRunning.current = day;
    // The answer goes into the chat of its own day, also when another day is on the screen.
    const set = (f: (c: ChatState) => ChatState) => setPlanChats((all) => ({ ...all, [day]: f(all[day] ?? EMPTY_CHAT) }));
    // Wait for the old messages first. Else the new turn can be in the
    // history and on the screen too, and it shows 2 times.
    return history
      .then(() => chat(set, false, text, files, (p) => api.askSide(`plan:${day}`, text.trim(), p, { model: pickArg(models.plan), files: files.map((f) => f.id) })))
      .finally(() => {
        planRunning.current = null;
      });
  };

  /** `key` is an item key, or "mission:<id>" for the chat of a mission. */
  const askItem = (key: string, text: string, files: api.FileRef[]) => {
    const set = (f: (c: ChatState) => ChatState) => setItemChats((all) => ({ ...all, [key]: f(all[key] ?? EMPTY_CHAT) }));
    text = withFiles(text, files);
    return chat(set, itemChats[key]?.busy ?? false, text, files, (p) => api.askSide(sideThread(key), text.trim(), p, { model: pickArg(models.item), files: files.map((f) => f.id) }));
  };

  // The chat of an item or a mission loads when its panel opens, one time. A message
  // that the user sent before the load came back stays after the old ones.
  // The questions on the home page load it too.
  const itemLoaded = useRef(new Set<string>());
  const loadItemChat = useCallback((key: string) => {
    if (itemLoaded.current.has(key)) return;
    itemLoaded.current.add(key);
    api
      .sideHistory(sideThread(key))
      .then((t) =>
        setItemChats((all) => {
          const c = all[key] ?? EMPTY_CHAT;
          return { ...all, [key]: { ...c, msgs: [...turnMsgs(t), ...c.msgs] } };
        }),
      )
      .catch(() => itemLoaded.current.delete(key));
  }, []);
  const chatKey = selMission !== null ? missionChatKey(selMission) : sel;
  useEffect(() => {
    if (chatKey) loadItemChat(chatKey);
  }, [chatKey, loadItemChat]);

  async function pickModel(box: "main" | "plan" | "item", m: string | null) {
    const before = models[box];
    setModels((all) => ({ ...all, [box]: m }));
    if (box !== "main") {
      store(`aifred-model-${box}`, m ?? "");
      return;
    }
    // The main pick is saved in the app, so the next start and the terminal use it too.
    try {
      await api.setChatModel(m);
    } catch (e) {
      setModels((all) => ({ ...all, main: before }));
      toast(`Cannot change the model: ${e}`, undefined, true);
    }
  }

  async function startNew() {
    try {
      await api.newConversation();
      setMain(EMPTY_CHAT);
      setMainOpen(false);
    } catch (e) {
      toast(String(e), undefined, true);
    }
  }

  const goTasks = () => {
    setPage("tasks");
    clearSel();
  };

  /** Opens the Calendar page on `day` (today when not given). */
  const goPlan = (day: string = today) => {
    setPage("plan");
    clearSel();
    setSide("chat");
    pickDay(day);
  };

  const setCalView = (v: "day" | "week") => {
    setCalViewState(v);
    store("aifred-cal-view", v);
  };

  const startPlan = (mode: "ai" | "self", day: string = today) => {
    goPlan(day);
    const p = plans[day] ?? null;
    if (mode === "self") {
      if (!plansReady(day)) return;
      run(api.setDayPlan(day, "self", p?.items ?? []), (np) => putPlan(day, np));
      return;
    }
    setPlanChatOpen(true);
    sendPlan(
      p?.mode === "review" && p.items.length
        ? "Plan my day. Keep the items that are in the plan now: I moved them here in a review."
        : p?.items.length
          ? "Plan my day again from the start."
          : "Plan my day.",
      [],
      day,
    );
  };

  const startOver = (day: string = calDay) => {
    run(Promise.all([api.clearDayPlan(day), api.clearSide(`plan:${day}`)]), () => {
      putPlan(day, null);
      setPlanChats((all) => ({ ...all, [day]: { ...EMPTY_CHAT, busy: all[day]?.busy ?? false } }));
    });
  };

  const open = (key: string) => {
    if (!byKey.has(key)) return;
    setSel(key);
    setSelMission(null);
    setSide("task");
    if (page !== "home" && page !== "plan" && page !== "tasks") setPage("home");
    requestAnimationFrame(() => document.getElementById(`row-${key}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  };
  /** Opens a mission of today on the right side, like an item. */
  const openMission = (id: number) => {
    setSelMission(id);
    setSel(null);
    setSide("task");
    setPage("home");
    requestAnimationFrame(() => document.getElementById(`mission-${id}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  };
  const close = () => {
    clearSel();
    setSide("chat");
  };

  // Escape closes the task panel, then the plan chat on a narrow window.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (sel || selMission !== null) close();
      else if (planChatOpen) setPlanChatOpen(false);
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  });

  // ---------- The width of the right side ----------

  const clampW = (w: number) => {
    const W = appRef.current?.clientWidth ?? 1400;
    const max = Math.max(300, W - 52 - 420);
    return Math.round(Math.min(max, Math.max(300, w)));
  };
  // A smaller window can make the saved width too wide. Draw again with the new limit.
  useEffect(() => {
    let t: number | undefined;
    const resize = () => {
      clearTimeout(t);
      t = window.setTimeout(() => setWinW(window.innerWidth), 120);
    };
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, []);
  const setW = (w: number | null) => {
    setSideW(w);
    store("aifred-side-w", w === null ? null : String(w));
    if (w === null) appRef.current?.style.removeProperty("--side-w");
  };
  const handle = (
    <div
      className="sz-h"
      role="separator"
      aria-orientation="vertical"
      aria-label="Width of the side panel. Drag, or use the arrow keys. Double-click to reset."
      tabIndex={0}
      onPointerDown={(e) => {
        if (e.button > 0) return;
        e.preventDefault();
        const start = { x: e.clientX, w: (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect().width };
        const el = e.currentTarget;
        el.setPointerCapture?.(e.pointerId);
        document.body.classList.add("sizing");
        let w = start.w;
        const move = (ev: PointerEvent) => {
          w = clampW(start.w + (start.x - ev.clientX));
          // Straight on the page while you drag, so React does not draw each step.
          appRef.current?.style.setProperty("--side-w", `${w}px`);
        };
        const end = () => {
          document.body.classList.remove("sizing");
          el.removeEventListener("pointermove", move);
          el.removeEventListener("pointerup", end);
          el.removeEventListener("pointercancel", end);
          setW(w);
        };
        el.addEventListener("pointermove", move);
        el.addEventListener("pointerup", end);
        el.addEventListener("pointercancel", end);
      }}
      onDoubleClick={() => setW(null)}
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const cur = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect().width;
        setW(clampW(cur + (e.key === "ArrowLeft" ? 20 : -20)));
      }}
    />
  );

  // ---------- Layout ----------

  const onPlan = page === "plan";
  const chatOn = onPlan ? true : page === "home" && mainOpen;
  const selItem = sel && (page === "home" || page === "tasks" || onPlan) ? byKey.get(sel) : undefined;
  // The missions show only on the home page.
  const selM = selMission !== null && page === "home" ? brief?.missions?.find((m) => m.id === selMission) : undefined;
  const selTitle = selItem?.title ?? selM?.title;
  const showSide = (v: "chat" | "task") => {
    setSide(v);
    if (v === "chat") {
      setUnread(false);
      if (onPlan) setPlanChatOpen(true);
    }
  };

  const tabs =
    selTitle !== undefined && chatOn ? (
      <div className="stabs" role="tablist" aria-label="Open on the right">
        <div
          className={`stab${side === "chat" ? " on" : ""}`}
          role="tab"
          aria-selected={side === "chat"}
          tabIndex={0}
          onClick={() => showSide("chat")}
          onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), showSide("chat"))}
        >
          {ICON.chat}
          <span className="lb">{onPlan ? "Plan chat" : "Conversation"}</span>
          {unread && side !== "chat" && <span className="new" title="New answer" />}
          {!onPlan && (
            <button
              className="x"
              aria-label="Close the conversation"
              onClick={(e) => {
                e.stopPropagation();
                setMainOpen(false);
                setSide("task");
                setUnread(false);
              }}
            >
              ✕
            </button>
          )}
        </div>
        <div
          className={`stab task${side === "task" ? " on" : ""}`}
          role="tab"
          aria-selected={side === "task"}
          tabIndex={0}
          onClick={() => showSide("task")}
          onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), showSide("task"))}
        >
          {ICON.task}
          <span className="lb">{selTitle}</span>
          <button
            className="x"
            aria-label={selM ? "Close the mission" : "Close the task"}
            onClick={(e) => {
              e.stopPropagation();
              close();
            }}
          >
            ✕
          </button>
        </div>
      </div>
    ) : null;

  // ---------- Corrections of the brief ----------

  /** You say that the brief is wrong about an item. The item loses its
   * reason at once. AIfred can learn a memory fact from it (learn.rs). */
  const correctBrief = useCallback(
    (i: Item, kind: api.CorrectionKind) => {
      if (!brief) return;
      // The correction is for today, also when the screen still shows an
      // older brief (today's brief is not written yet). So "Not today"
      // counts for today, and today's brief gets it.
      const day = today;
      const shown = brief.day;
      // A tree row can show the reason of one of its subtasks: the
      // correction goes to that item. The backend corrects only this one
      // item, so the screen drops only its reason and its missions. The other
      // items of the tree keep theirs.
      const key = i.whyKey ?? i.key;
      const title = i.linked?.find((l) => l.key === key)?.title ?? i.title;
      const drop = <T,>(r: Record<string, T> | undefined) => r && Object.fromEntries(Object.entries(r).filter(([k]) => k !== key));
      run(api.correctBrief(key, kind, day), (c) => {
        // Like missions::today: a mission with the corrected item leaves the list.
        setBrief((b) =>
          b && b.day === shown
            ? { ...b, why: drop(b.why)!, goal: drop(b.goal), missions: b.missions?.filter((m) => !m.items.some((x) => x.key === key)) }
            : b,
        );
        if (c.learned) api.memoryFacts().then(setFacts).catch(() => {});
        const undo = () =>
          run(api.undoCorrection(c.correction.id), () => {
            reloadBrief(shown);
            if (c.learned) api.memoryFacts().then(setFacts).catch(() => {});
          });
        const text = c.learned
          ? `AIfred learned: ${c.learned.text}`
          : kind === "not_today"
            ? `AIfred will not count this for today: ${title}`
            : `AIfred will rank this lower from now on: ${title}`;
        toast(text, undo);
      });
    },
    [brief, today, run, toast, reloadBrief],
  );

  // ---------- Missions (missions.rs) ----------

  /** Puts a changed mission into the brief on the screen. */
  const putMission = useCallback((m: api.Mission) => {
    setBrief((b) => (b?.missions ? { ...b, missions: b.missions.map((x) => (x.id === m.id ? m : x)) } : b));
  }, []);

  /** Ticks a step of the mission, or takes the tick away. The items do not change.
   * A second click on the same step waits for the first answer: else both
   * clicks send the same value, from the same step on the screen. */
  const stepBusy = useRef(new Set<string>());
  const missionStep = useCallback(
    (m: api.Mission, index: number) => {
      const k = `${m.id}:${index}`;
      if (stepBusy.current.has(k)) return;
      stepBusy.current.add(k);
      void run(api.setMissionStep(m.id, index, !m.checklist[index]?.done), putMission).finally(() => stepBusy.current.delete(k));
    },
    [run, putMission],
  );

  /** You say that the brief is wrong about a mission: a correction of each
   * of its open items. The mission leaves the list at once. */
  const correctMission = useCallback(
    (m: api.Mission, kind: api.CorrectionKind) => {
      if (!brief) return;
      // As for one item: the correction is for today (see correctBrief).
      const day = today;
      const shown = brief.day;
      // Like missions::correct: only the open items get a correction, so
      // only they lose their reason. A closed item keeps its reason.
      const keys = new Set(m.items.filter((i) => i.open).map((i) => i.key));
      const drop = <T,>(r: Record<string, T> | undefined) => r && Object.fromEntries(Object.entries(r).filter(([k]) => !keys.has(k)));
      run(api.correctMission(m.id, kind, day), (cs) => {
        setBrief((b) =>
          b && b.day === shown ? { ...b, missions: b.missions?.filter((x) => x.id !== m.id), why: drop(b.why)!, goal: drop(b.goal) } : b,
        );
        const learned = cs.map((c) => c.learned).filter((f) => !!f);
        if (learned.length) api.memoryFacts().then(setFacts).catch(() => {});
        const undo = () =>
          run(
            cs.reduce<Promise<unknown>>((p, c) => p.then(() => api.undoCorrection(c.correction.id)), Promise.resolve()),
            () => {
              reloadBrief(shown);
              if (learned.length) api.memoryFacts().then(setFacts).catch(() => {});
            },
          );
        const text = learned.length
          ? `AIfred learned: ${learned[learned.length - 1]!.text}`
          : kind === "not_today"
            ? `AIfred will not count this for today: ${m.title}`
            : `AIfred will rank this lower from now on: ${m.title}`;
        toast(text, undo);
      });
    },
    [brief, today, run, toast, reloadBrief],
  );

  /** You add an item to a mission or take one out. The item can come from
   * or go to another mission, so the brief loads again. The undo puts the
   * item back where it was. */
  const changeMission = useCallback(
    (change: Promise<api.MissionChanged>, text: string) => {
      const shown = brief?.day;
      return run(change, (c) => {
        putMission(c.mission);
        reloadBrief(shown);
        toast(text, () => run(api.restoreMissionItem(c.before), () => reloadBrief(shown)));
      });
    },
    [brief, run, putMission, reloadBrief, toast],
  );
  const addToMission = useCallback(
    (m: api.Mission, key: string) =>
      changeMission(api.addMissionItem(m.id, key), `Added to ${m.title}: ${byKey.get(key)?.title ?? key}. AIfred keeps it there.`),
    [changeMission, byKey],
  );
  const removeFromMission = useCallback(
    (m: api.Mission, key: string) =>
      changeMission(
        api.removeMissionItem(m.id, key),
        `Taken out of ${m.title}: ${m.items.find((i) => i.key === key)?.title ?? key}. AIfred does not put it back.`,
      ),
    [changeMission],
  );

  /** First the items in `close` close, one at a time, each with its own
   * command. GitHub items cannot close here: AIfred only reads GitHub. When
   * all of them closed, the mission is done. Its one undo opens the mission
   * and the closed tasks and emails again. The app shows one toast at a
   * time, so each item cannot have its own toast. A ClickUp task keeps its
   * own undo in the list of closed ClickUp tasks. When one item did not
   * close, the mission stays open. */
  const finishMission = useCallback(
    async (m: api.Mission, close: string[]) => {
      const items = close.map((k) => byKey.get(k)).filter((i): i is Item => !!i && !i.done && !i.gh);
      const undos: (() => void)[] = [];
      const undoItems = () => undos.forEach((u) => u());
      for (const i of items) {
        if (!(await toggleDone(i, (u) => undos.push(u)))) {
          // The failed close showed its reason a moment ago. Keep it in the text.
          clearTimeout(toastTimer.current);
          setToast((t) => ({
            text: `The mission stays open: AIfred could not close ${i.title}.${t?.error ? ` ${t.text}` : ""}${undos.length ? " Undo opens the items that closed." : ""}`,
            undo: undos.length ? undoItems : undefined,
            error: true,
          }));
          toastTimer.current = window.setTimeout(() => setToast(null), 8000);
          return false;
        }
      }
      return run(api.finishMission(m.id), () => {
        setBrief((b) => (b?.missions ? { ...b, missions: b.missions.filter((x) => x.id !== m.id) } : b));
        const shown = brief?.day;
        // The undo opens the mission and its closed tasks and emails again.
        // A closed ClickUp task stays closed: open it in the list of
        // closed ClickUp tasks.
        const undo = () =>
          run(api.reopenMission(m.id), () => {
            undoItems();
            reloadBrief(shown);
          });
        const n = undos.length;
        toast(n ? `Mission done: ${m.title}. Undo opens it and its ${n === 1 ? "closed item" : `${n} closed items`} again.` : `Mission done: ${m.title}`, undo);
      });
    },
    [run, byKey, toggleDone, toast, brief, reloadBrief],
  );

  // ---------- Subtasks (links.rs) ----------

  const titleOf = useCallback((key: string) => byKey.get(key)?.title ?? key, [byKey]);

  /** A move takes the subtasks of the item with it (links.rs). So the undo
   * only puts the item back under its old parent, or out again. It also
   * puts back the sort marks: the AI sort can move the items again, as
   * before the move. */
  const linkItems = useCallback(
    (main: string, key: string) =>
      run(api.linkItems(main, key), (c) => {
        setLinks({ links: c.links });
        toast(`${titleOf(key)} is now a subtask of ${titleOf(main)}`, () => run(api.restoreLink(c.before), setLinks));
      }),
    [run, toast, titleOf],
  );

  const unlinkItem = useCallback(
    (key: string) =>
      run(api.unlinkItem(key), (c) => {
        setLinks({ links: c.links });
        toast(`Moved out: ${titleOf(key)}`, c.before.link ? () => run(api.restoreLink(c.before), setLinks) : undefined);
      }),
    [run, toast, titleOf],
  );

  // ---------- Blockers (blockers.rs) ----------

  /** Says that `key` waits for `blocker`, with an undo. */
  const addBlocker = useCallback(
    (key: string, blocker: string) =>
      run(api.addBlocker(key, blocker), (c) => {
        setBlockers({ blockers: c.blockers });
        toast(`${titleOf(key)} now waits for ${titleOf(blocker)}`, () => run(api.restoreBlocker(c.before), setBlockers));
      }),
    [run, toast, titleOf],
  );

  /** Takes away the link, with an undo. AIfred does not add it again. */
  const removeBlocker = useCallback(
    (key: string, blocker: string) =>
      run(api.removeBlocker(key, blocker), (c) => {
        setBlockers({ blockers: c.blockers });
        toast(`${titleOf(key)} does not wait for ${titleOf(blocker)} now`, () => run(api.restoreBlocker(c.before), setBlockers));
      }),
    [run, toast, titleOf],
  );

  /** Undoes AIfred's moves from the brief. The lines go away from the brief. */
  const undoMoves = useCallback(
    (moves: api.Moved[]) => {
      const ids = moves.map((m) => m.id);
      return run(api.undoMoves(ids), (l) => {
        setLinks(l);
        setBrief((b) => (b?.moves ? { ...b, moves: b.moves.filter((m) => !ids.includes(m.id)) } : b));
        const names = moves.map((m) => m.title).join(", ");
        toast(`Put back: ${names}. AIfred does not move ${moves.length > 1 ? "them" : "it"} again.`);
      });
    },
    [run, toast],
  );

  // An item that you work on can close elsewhere, for example when AIfred
  // finishes the task in a chat. Then its work stops, with its real time.
  // The check runs again on each new list, and the timer goes only after
  // the save. So it remembers the item that it finished: one save and one
  // toast for each item. After a failed save, the timer stays on screen
  // and you click Done. The error does not come back with each new list.
  const autoFinished = useRef<string | null>(null);
  useEffect(() => {
    if (!work || !byKey.get(work.key)?.done) {
      autoFinished.current = null;
      return;
    }
    if (autoFinished.current === work.key) return;
    autoFinished.current = work.key;
    void afterFinish(work.key, null);
  }, [work, byKey, afterFinish]);

  const ctx: AppCtx = {
    google,
    today,
    items,
    byKey,
    settings: cfg,
    plan,
    planItems,
    sched,
    plans,
    week,
    events: calendar.events,
    calDay,
    pickDay,
    calView,
    setCalView,
    brief,
    reloadBrief,
    goals,
    goalText,
    changeGoals,
    briefState,
    // The first loads of the day are not back yet: the home page waits, so
    // that it does not show the task list for a moment before the questions.
    // A later reload of a list (the timer, or after an answer) keeps the
    // page: else the question card and the text in its box go away.
    homeLoading:
      briefLoaded !== today ||
      (!tasksLoaded && !tasksError) ||
      ((brief?.ask?.length ?? 0) > 0 && ((githubState === "loading" && !githubBack) || (clickupState === "loading" && !clickupBack))),
    githubState,
    clickupState,
    githubWarning: github.warning,
    clickupWarning: clickup.warning,
    mailState,
    mailWarning: mail.warning,
    sentMail: mail.sent,
    waiting,
    waitingError,
    links,
    linkItems,
    unlinkItem,
    blockers,
    addBlocker,
    removeBlocker,
    undoMoves,
    trackEmail,
    hideSent,
    addWaiting,
    changeWaiting,
    removeWaiting,
    tasksError,
    sel,
    open,
    close,
    main,
    mainOpen,
    openMain: () => {
      setPage("home");
      setMainOpen(true);
      showSide("chat");
    },
    sendMain,
    planChat,
    sendPlan,
    itemChat: (key) => itemChats[key] ?? EMPTY_CHAT,
    selMission,
    openMission,
    askItem,
    loadItemChat,
    models,
    setModel: pickModel,
    toggleDone,
    work,
    startWork,
    stopWork,
    finishItem,
    correctBrief,
    missionStep,
    correctMission,
    finishMission,
    addToMission,
    removeFromMission,
    addTask,
    addSteps,
    homeMode,
    setHomeMode,
    editTask,
    tickStep,
    deleteTask,
    tomorrow,
    setMinutes,
    savePlan,
    addToPlan,
    removeFromPlan,
    setPlanTime,
    moveToDay,
    startPlan,
    startOver,
    writeBrief,
    toast,
    reviewCards,
    setReviewCard: (card, mode) => setReviewCards((c) => ({ ...c, [card]: mode })),
    reload: () => {
      reloadAfterAnswer();
      loadWaiting();
    },
    reloadClickup: () => loadClickup(false, clickupOn),
    talk,
    setTalk,
    skipped,
    setSkipped,
    noteAnswer,
    goPlan: () => goPlan(),
    goTasks,
  };

  // Open links in the system browser, not inside the app window.
  function onClick(e: React.MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (a?.href.startsWith("http")) {
      e.preventDefault();
      openUrl(a.href);
    }
  }

  const sync = (() => {
    const parts: React.ReactNode[] = [];
    const one = (name: string, state: ListState, at: number, warn?: string) => {
      if (state === "off") return;
      if (state === "loading") parts.push(<span key={name}>{name}…</span>);
      else if (state !== "ok") parts.push(<span key={name} className="err" title={state}>{name}: error</span>);
      else parts.push(<span key={name} className={warn ? "warn" : ""} title={warn}>{name} {at ? agoSecs(at) : ""}</span>);
    };
    one("GitHub", githubState, github.fetched_at, github.warning);
    one("ClickUp", clickupState, clickup.fetched_at, clickup.warning);
    one("Calendar", calendarState, calendar.fetched_at, calendar.warning);
    one("Gmail", mailState, mail.fetched_at, mail.warning);
    return parts;
  })();
  const states = [...Object.values(sourceStates), calendarState];
  const loading = states.includes("loading");
  const bad = states.some((s) => !["loading", "ok", "off"].includes(s));

  const rail = (k: Page | "chat", icon: React.ReactNode, label: string, onClick: () => void, on: boolean, dot?: boolean) => (
    <button key={k} className={on ? "on" : ""} title={label} aria-label={label} aria-pressed={on} onClick={onClick}>
      {icon}
      {dot && <span className="new" />}
    </button>
  );

  return (
    <Ctx.Provider value={ctx}>
      <div
        ref={appRef}
        className={`app ${onPlan ? "plan" : "home"}`}
        style={sideW ? ({ "--side-w": `${clampW(sideW)}px` } as React.CSSProperties) : undefined}
        onClick={onClick}
      >
        <div className="chrome">
          <span className="logo" aria-hidden />
          <span className="name">AIfred</span>
          <span className="hide-s">{info?.backend ?? ""}</span>
          <span className="sync" role="status">
            <span className={`ok${loading ? " busy" : bad ? " bad" : ""}`} />
            {sync.map((p, i) => (
              <span key={i} className={i ? "hide-s" : ""}>
                {i ? "· " : ""}
                {p}
              </span>
            ))}
            <button
              className="reload"
              title="Reload the list"
              aria-label="Reload the list"
              onClick={() => {
                loadTasks();
                loadGithub(true);
                loadClickup(true, clickupOn);
                loadCalendar(true);
                loadMail(true);
                loadWaiting();
              }}
            >
              ↻
            </button>
          </span>
        </div>
        <div className="body">
          <nav className="rail" aria-label="App">
            {rail("home", ICON.home, "Home", () => (setPage("home"), clearSel()), page === "home")}
            {rail("plan", ICON.cal, "Calendar", () => goPlan(), onPlan)}
            {rail("tasks", ICON.task, "Tasks", goTasks, page === "tasks")}
            {rail("chat", ICON.chat, "Conversation", ctx.openMain, page === "home" && mainOpen)}
            {rail("memory", ICON.mem, "What AIfred knows about you", () => (setPage("memory"), clearSel()), page === "memory")}
            <span className="sp" />
            {rail("settings", ICON.gear, "Settings", () => (setPage("settings"), clearSel()), page === "settings", update?.state === "ready")}
          </nav>

          {page === "home" && (
            <div className="home-col">
              <div className="banners">
                {loadError && <div className="banner err">{loadError}</div>}
                {info?.api_key_set && <div className="banner err">ANTHROPIC_API_KEY is set: Claude uses the API key, not your login.</div>}
                {update?.state === "ready" && (
                  <div className="banner">
                    <span>AIfred {update.latest} is ready.</span>
                    <RestartButton small busy={anyBusy} onError={setRestartError} />
                    {restartError && <span>{restartError}</span>}
                  </div>
                )}
              </div>
              <Home />
            </div>
          )}
          {onPlan && <CalendarView onChat={() => (setPlanChatOpen(true), showSide("chat"))} />}
          {page === "settings" && settings && (
            <div className="scroll page-main page">
              <div className="page-inner">
                <SettingsPage
                  settings={settings}
                  onSaved={setSettings}
                  info={info}
                  clickupToken={clickupToken}
                  onClickupTokenChanged={() => checkClickup(true)}
                  onGoogleChanged={() => {
                    loadGoogle();
                    loadCalendar(true);
                    loadMail(true);
                  }}
                  update={update}
                  onUpdateChanged={setUpdate}
                  busy={anyBusy}
                />
              </div>
            </div>
          )}
          {page === "tasks" && (
            <div className="scroll page-main page">
              <div className="page-inner">
                <TasksPage />
              </div>
            </div>
          )}
          {page === "memory" && (
            <div className="scroll page-main page">
              <div className="page-inner">
                <h2 className="title">What AIfred knows about you</h2>
                <p className="note-line" style={{ paddingTop: 0 }}>
                  AIfred uses these facts in your chats. It also adds and forgets facts when you tell it to.
                </p>
                <MemoryList
                  facts={facts}
                  onAdd={(f) => setFacts((all) => [...all, f])}
                  onChange={(f) => setFacts((all) => all.map((x) => (x.id === f.id ? f : x)))}
                  onDelete={(id) => setFacts((all) => all.filter((x) => x.id !== id))}
                />
              </div>
            </div>
          )}

          {selItem && (
            <Detail key={selItem.key} item={selItem} hidden={chatOn && side === "chat"} tabs={tabs} handle={handle} />
          )}
          {selM && <MissionDetail key={selM.id} m={selM} hidden={chatOn && side === "chat"} tabs={tabs} handle={handle} />}
          {page === "home" && mainOpen && (
            <aside className="chat" aria-label="Conversation" hidden={selTitle !== undefined && side === "task"}>
              {handle}
              {tabs}
              <div className="chat-h">
                Conversation
                <span className="sp" />
                <button className="linkbtn" onClick={startNew} disabled={main.busy}>
                  New
                </button>
                <button className="linkbtn" title="Close the conversation panel" onClick={() => setMainOpen(false)}>
                  Close
                </button>
              </div>
              <MainMessages />
            </aside>
          )}
          {onPlan && (
            <aside className={`chat${planChatOpen ? " open" : ""}`} aria-label="Plan chat" hidden={selTitle !== undefined && side === "task"}>
              {handle}
              {tabs}
              <div className="chat-h">
                Plan chat
                <span className="sp" />
                <button className="linkbtn m-only" onClick={() => setPlanChatOpen(false)}>
                  Close
                </button>
              </div>
              <PlanChat />
            </aside>
          )}
        </div>
        <div className="toasts">
        {closed.map((c) => (
          <div key={c.id} className="toast" role="status">
            <span>Closed in ClickUp: {c.title}</span>
            <button aria-label={`Undo: ${c.title}`} onClick={() => undoClose(c)}>
              Undo
            </button>
            <button aria-label={`Hide this message: ${c.title}`} onClick={() => setClosed((l) => l.filter((x) => x.id !== c.id))}>
              ✕
            </button>
          </div>
        ))}
        {toastState && (
          <div className={`toast${toastState.error ? " err" : ""}`} role={toastState.error ? "alert" : "status"}>
            <span>{toastState.text}</span>
            {toastState.undo && (
              <button
                onClick={() => {
                  toastState.undo!();
                  setToast(null);
                }}
              >
                Undo
              </button>
            )}
            <button aria-label="Hide this message" onClick={() => setToast(null)}>
              ✕
            </button>
          </div>
        )}
        </div>
      </div>
    </Ctx.Provider>
  );
}

/** The messages of the main conversation, with the chat box under them. */
function MainMessages() {
  const { main, sendMain, models, setModel, open, byKey } = useApp();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.scrollTo({ top: ref.current.scrollHeight });
  }, [main.msgs.length, main.busy, main.progress.length]);
  return (
    <>
      <div className="msgs" ref={ref}>
        <Messages list={main.msgs} busy={main.busy} progress={main.progress} onItem={open} hasItem={(k) => byKey.has(k)} />
      </div>
      <div className="chat-in">
        <Composer
          id="main-panel"
          placeholder="Ask AIfred…"
          busy={main.busy}
          model={models.main}
          onModel={(m) => setModel("main", m)}
          onSend={(text, files) => sendMain(text, files)}
        />
      </div>
    </>
  );
}
