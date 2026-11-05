import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "./api";
import { Composer, ICON, Messages, type ChatMsg } from "./Chat";
import { Ctx, useApp, type AppCtx, type ChatState, type ListState } from "./ctx";
import { Detail } from "./Detail";
import { Home } from "./Home";
import { addDays, buildItems, dayOf, schedule, type Item } from "./items";
import { JobsPage } from "./Jobs";
import { MemoryList } from "./Memory";
import { PlanChat, PlanView } from "./Plan";
import { RestartButton, SettingsPage } from "./Settings";

type Page = "home" | "plan" | "memory" | "jobs" | "settings";
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

const turnMsgs = (turns: (api.Turn | api.SideTurn)[]): ChatMsg[] =>
  turns.map((t) => ({ role: t.role, text: t.text, files: t.files, info: "info" in t ? t.info : undefined }));

export default function App() {
  const today = useToday();
  const [page, setPage] = useState<Page>("home");
  const [info, setInfo] = useState<api.Info | null>(null);
  const [loadError, setLoadError] = useState("");
  const [facts, setFacts] = useState<api.Fact[]>([]);
  const [tasks, setTasks] = useState<api.Task[]>([]);
  const [tasksError, setTasksError] = useState("");
  const [tasksLoaded, setTasksLoaded] = useState(false);
  const [github, setGithub] = useState<api.GithubList>({ items: [], fetched_at: 0 });
  const [githubState, setGithubState] = useState<ListState>("loading");
  const [clickup, setClickup] = useState<api.ClickupList>({ items: [], fetched_at: 0 });
  const [clickupState, setClickupState] = useState<ListState>("loading");
  const [clickupToken, setClickupToken] = useState<api.ClickupTokenStatus | null>(null);
  const [settings, setSettings] = useState<api.Settings | null>(null);
  const [update, setUpdate] = useState<api.UpdateStatus | null>(null);
  const [restartError, setRestartError] = useState("");
  const [meta, setMeta] = useState<api.ItemMeta[]>([]);
  const [est, setEst] = useState<Record<string, api.Estimate>>({});
  const [brief, setBrief] = useState<api.Brief | null>(null);
  const [briefLoaded, setBriefLoaded] = useState("");
  const [briefState, setBriefState] = useState("idle");
  // The mode of the home box stays when you go to another page.
  const [homeMode, setHomeMode] = useState<"ask" | "task">("ask");
  const [plan, setPlan] = useState<api.DayPlan | null>(null);
  // The day whose plan is loaded. Until today's plan loads, a change must
  // not save: it would use the plan of the day before.
  const [planLoaded, setPlanLoaded] = useState("");
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
  const [planChat, setPlanChat] = useState<ChatState>(EMPTY_CHAT);
  const [planChatOpen, setPlanChatOpen] = useState(false);
  /** The day of the plan chat on the screen. */
  const planDay = useRef("");
  const [itemChats, setItemChats] = useState<Record<string, ChatState>>({});
  // A restart to update must wait for every running answer and the brief.
  const anyBusy = main.busy || planChat.busy || Object.values(itemChats).some((c) => c.busy) || briefState === "writing";
  const [sel, setSel] = useState<string | null>(null);
  const [side, setSide] = useState<"chat" | "task">("chat");
  const [unread, setUnread] = useState(false);
  const [toastState, setToast] = useState<Toast | null>(null);
  const [closed, setClosed] = useState<Closed[]>([]);
  /** The ClickUp tasks that are closing now. A second click on one does nothing. */
  const closing = useRef(new Set<string>());
  const [sideW, setSideW] = useState<number | null>(() => {
    const w = Number(stored("aifred-side-w"));
    return w >= 300 ? w : null;
  });
  const [, setWinW] = useState(0);
  const appRef = useRef<HTMLDivElement>(null);
  const githubLoad = useRef(0);
  const clickupLoad = useRef(0);
  const briefTried = useRef(new Set<string>());
  const now = useRef({ sel, side });
  now.current = { sel, side };

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

  const loadClickup = useCallback(
    (force: boolean, connected: boolean) => {
      const n = ++clickupLoad.current;
      if (!connected) {
        setClickup({ items: [], fetched_at: 0 });
        setClickupState("off");
        return;
      }
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
    loadTasks();
    loadMeta();
    loadGithub(false);
    checkClickup(false);
  }, [loadTasks, loadMeta, loadGithub, checkClickup]);

  // The plan, the brief, and the plan chat belong to one day. A new day loads them again.
  useEffect(() => {
    let live = true;
    planDay.current = today;
    setPlanChat(EMPTY_CHAT);
    setPlan(null);
    api
      .dayPlan(today)
      .then((p) => {
        if (!live) return;
        setPlan(p);
        setPlanLoaded(today);
      })
      .catch(() => {});
    api
      .brief(today)
      .then((b) => {
        if (!live) return;
        setBrief(b);
        setBriefLoaded(today);
      })
      .catch(() => live && setBriefLoaded(today));
    api
      .sideHistory(`plan:${today}`)
      .then((t) => live && setPlanChat((c) => ({ ...c, msgs: [...turnMsgs(t), ...c.msgs] })))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [today]);

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
    }, refreshMinutes * 60_000);
    return () => clearInterval(timer);
  }, [refreshMinutes, loadGithub, loadClickup, clickupOn]);

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

  const writeBrief = useCallback(() => {
    const day = today;
    briefTried.current.add(day);
    setBriefState("writing");
    api
      .writeBrief(day)
      .then((b) => {
        if (b.day === dayOf(new Date())) setBrief(b);
        setBriefState("idle");
        api.estimates().then(setEst).catch(() => {});
      })
      .catch((e) => setBriefState(String(e)));
  }, [today]);

  // The brief writes itself once a day, after the lists are loaded, when the setting is on.
  const listsReady = githubState !== "loading" && clickupState !== "loading";
  useEffect(() => {
    if (!settings?.auto_brief || briefLoaded !== today || brief || !listsReady || briefTried.current.has(today)) return;
    writeBrief();
  }, [settings?.auto_brief, briefLoaded, today, brief, listsReady, writeBrief]);

  // ---------- Items ----------

  const items = useMemo(
    () => buildItems(tasks, github.items, clickup.items, meta, est, brief?.why ?? {}),
    [tasks, github.items, clickup.items, meta, est, brief],
  );
  const byKey = useMemo(() => new Map(items.map((i) => [i.key, i])), [items]);
  const planItems = useMemo(() => (plan?.items ?? []).map((p) => byKey.get(p.key)).filter((i): i is Item => !!i), [plan, byKey]);
  const cfg = settings ?? api.DEFAULT_SETTINGS;
  const sched = useMemo(() => schedule(planItems, cfg), [planItems, cfg]);

  // An item that left the list (closed in ClickUp, deleted) closes its panel.
  useEffect(() => {
    if (sel && !byKey.has(sel) && githubState !== "loading" && clickupState !== "loading") setSel(null);
  }, [sel, byKey, githubState, clickupState]);

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

  // ---------- Actions on items ----------

  const putTask = (t: api.Task) => setTasks((all) => all.map((x) => (x.id === t.id ? t : x)));

  const toggleDone = useCallback(
    (i: Item) => {
      if (i.task) {
        const done = !i.done;
        run(api.setTaskDone(i.task.id, done), (t) => {
          putTask(t);
          if (done) toast(`Finished: ${i.title}`, () => run(api.setTaskDone(t.id, false), putTask));
        });
      } else if (i.cu) {
        const c = i.cu;
        if (closing.current.has(c.id)) return;
        closing.current.add(c.id);
        // This changes the real task in ClickUp, so there is an undo.
        run(api.clickupMarkDone(c.id), (before) => {
          setClickup((l) => ({ ...l, items: l.items.filter((x) => x.id !== c.id) }));
          setClosed((l) => [...l.filter((x) => x.id !== c.id), { id: c.id, title: c.title, before }]);
        }).finally(() => closing.current.delete(c.id));
      }
    },
    [run, toast],
  );

  const undoClose = useCallback(
    (c: Closed) =>
      run(api.clickupUndoDone(c.id, c.before), () => {
        setClosed((l) => l.filter((x) => x.id !== c.id));
        loadClickup(true, true);
      }),
    [run, loadClickup],
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
        setTasks((all) => all.filter((x) => x.id !== t.id));
        if (now.current.sel === i.key) setSel(null);
        toast(`Deleted: ${t.title}`);
      });
    },
    [run, toast],
  );

  // ---------- The plan ----------

  // Only a plan of the day on screen may change the screen. An answer
  // that comes back after midnight belongs to the old day.
  const putPlan = useCallback((day: string, p: api.DayPlan | null) => {
    if (planDay.current === day) setPlan(p);
  }, []);

  /** True when the list of the item is loaded, so an item that is not in it is gone. */
  const listLoaded = useCallback(
    (key: string) => {
      if (key.startsWith("task:")) return tasksLoaded;
      if (key.startsWith("github:")) return githubState === "ok";
      if (key.startsWith("clickup:")) return clickupState === "ok" || clickupState === "off";
      return true;
    },
    [tasksLoaded, githubState, clickupState],
  );

  /**
   * Saves the plan with `keys` (the items on screen) in this order. A plan
   * item that is not on screen because its list did not load yet keeps its
   * place, so the save does not delete it.
   */
  const savePlan = useCallback(
    (keys: string[], text?: string) => {
      const day = today;
      if (planLoaded !== day) {
        toast("The plan for today is still loading. Try again in a moment.", undefined, true);
        return;
      }
      const before = plan;
      const old = (plan?.items ?? []).map((p) => p.key);
      const shown = new Set(keys);
      const hidden = new Set(old.filter((k) => !shown.has(k) && !byKey.has(k) && !listLoaded(k)));
      const merged: string[] = [];
      let n = 0;
      for (const k of old) {
        if (hidden.has(k)) merged.push(k);
        else if (n < keys.length) merged.push(keys[n++]);
      }
      merged.push(...keys.slice(n));
      const why = new Map((plan?.items ?? []).map((p) => [p.key, p.why]));
      const next: api.PlanItem[] = merged.map((key) => (why.get(key) ? { key, why: why.get(key) } : { key }));
      const mode = plan?.mode ?? "self";
      setPlan({ day, mode, items: next, updated: Date.now() / 1000 });
      api
        .setDayPlan(day, mode, next)
        .then((p) => {
          putPlan(day, p);
          if (text) {
            toast(text, () => {
              const old = before?.items ?? [];
              putPlan(day, before);
              if (before) api.setDayPlan(day, before.mode, old).then((p) => putPlan(day, p)).catch(() => {});
              else api.clearDayPlan(day).catch(() => {});
            });
          }
        })
        .catch((e) => {
          putPlan(day, before);
          toast(`The plan is not saved: ${e}`, undefined, true);
        });
    },
    [plan, planLoaded, today, toast, byKey, listLoaded, putPlan],
  );

  const planKeys = useMemo(() => planItems.map((i) => i.key), [planItems]);
  const addToPlan = useCallback(
    (key: string, at?: number) => {
      const keys = planKeys.filter((k) => k !== key);
      keys.splice(at ?? keys.length, 0, key);
      savePlan(keys, `Added to today's plan: ${byKey.get(key)?.title ?? ""}`);
    },
    [planKeys, savePlan, byKey],
  );
  const removeFromPlan = useCallback(
    (key: string) => savePlan(planKeys.filter((k) => k !== key), `Taken out of the plan: ${byKey.get(key)?.title ?? ""}`),
    [planKeys, savePlan, byKey],
  );

  const tomorrow = useCallback(
    (i: Item) => {
      const t = i.task;
      if (!t) return;
      const next = addDays(today, 1);
      const before = { due: t.due ?? "", start: t.start ?? "" };
      const changes: api.TaskChanges = { due: next, ...(t.start && t.start > next ? { start: "" } : {}) };
      const wasIn = planKeys.includes(i.key);
      run(api.editTask(t.id, changes), (nt) => {
        putTask(nt);
        if (wasIn) savePlan(planKeys.filter((k) => k !== i.key));
        toast(`Moved to tomorrow: ${t.title}`, () => {
          run(api.editTask(t.id, before), putTask);
          if (wasIn) savePlan(planKeys);
        });
      });
    },
    [today, planKeys, run, savePlan, toast],
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
    api.dayPlan(today).then((p) => putPlan(today, p)).catch(() => {});
  }, [loadTasks, loadMeta, today, putPlan]);

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
    try {
      const a = await ask((e) => set((c) => ({ ...c, progress: [...c.progress, api.describe(e)] })));
      set((c) => ({ ...c, msgs: [...c.msgs, { role: "assistant", text: a.text, info: a.info, facts: a.memory }] }));
    } catch (e) {
      set((c) => ({ ...c, msgs: [...c.msgs, { role: "error", text: String(e) }] }));
    } finally {
      set((c) => ({ ...c, busy: false, progress: [] }));
      reloadAfterAnswer();
    }
    if (now.current.sel && now.current.side === "task") setUnread(true);
    return true;
  }

  const sendMain = (text: string, files: api.FileRef[]) => {
    setMainOpen(true);
    setSide("chat");
    if (page !== "home") setPage("home");
    text = withFiles(text, files);
    const model = mainModelKnown.current ? pickArg(models.main) : undefined;
    return chat(setMain, main.busy, text, files, (p) => api.ask(text.trim(), p, { model, files: files.map((f) => f.id) }));
  };

  const sendPlan = (text: string, files: api.FileRef[] = []) => {
    setSide("chat");
    text = withFiles(text, files);
    // An answer that comes back after midnight belongs to the old day's
    // chat (the backend saved it there), so it does not go into the new one.
    const day = today;
    const set = (f: (c: ChatState) => ChatState) => setPlanChat((c) => (planDay.current === day ? f(c) : c));
    return chat(set, planChat.busy, text, files, (p) => api.askSide(`plan:${today}`, text.trim(), p, { model: pickArg(models.plan), files: files.map((f) => f.id) }));
  };

  const askItem = (key: string, text: string, files: api.FileRef[]) => {
    const set = (f: (c: ChatState) => ChatState) => setItemChats((all) => ({ ...all, [key]: f(all[key] ?? EMPTY_CHAT) }));
    text = withFiles(text, files);
    return chat(set, itemChats[key]?.busy ?? false, text, files, (p) => api.askSide(`item:${key}`, text.trim(), p, { model: pickArg(models.item), files: files.map((f) => f.id) }));
  };

  // The chat of an item loads when its panel opens, one time. A message
  // that the user sent before the load came back stays after the old ones.
  const itemLoaded = useRef(new Set<string>());
  useEffect(() => {
    if (!sel || itemLoaded.current.has(sel)) return;
    const key = sel;
    itemLoaded.current.add(key);
    api
      .sideHistory(`item:${key}`)
      .then((t) =>
        setItemChats((all) => {
          const c = all[key] ?? EMPTY_CHAT;
          return { ...all, [key]: { ...c, msgs: [...turnMsgs(t), ...c.msgs] } };
        }),
      )
      .catch(() => itemLoaded.current.delete(key));
  }, [sel]);

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

  const goPlan = () => {
    setPage("plan");
    setSel(null);
    setSide("chat");
  };

  const startPlan = (mode: "ai" | "self") => {
    goPlan();
    if (mode === "self") {
      if (planLoaded !== today) {
        toast("The plan for today is still loading. Try again in a moment.", undefined, true);
        return;
      }
      const day = today;
      run(api.setDayPlan(day, "self", plan?.items ?? []), (p) => putPlan(day, p));
      return;
    }
    setPlanChatOpen(true);
    sendPlan(plan?.items.length ? "Plan my day again from the start." : "Plan my day.");
  };

  const startOver = () => {
    run(Promise.all([api.clearDayPlan(today), api.clearSide(`plan:${today}`)]), () => {
      setPlan(null);
      setPlanChat(EMPTY_CHAT);
    });
  };

  const open = (key: string) => {
    if (!byKey.has(key)) return;
    setSel(key);
    setSide("task");
    if (page !== "home" && page !== "plan") setPage("home");
    requestAnimationFrame(() => document.getElementById(`row-${key}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  };
  const close = () => {
    setSel(null);
    setSide("chat");
  };

  // Escape closes the task panel, then the plan chat on a narrow window.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (sel) close();
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
  const selItem = sel && (page === "home" || onPlan) ? byKey.get(sel) : undefined;
  const showSide = (v: "chat" | "task") => {
    setSide(v);
    if (v === "chat") {
      setUnread(false);
      if (onPlan) setPlanChatOpen(true);
    }
  };

  const tabs =
    selItem && chatOn ? (
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
          <span className="lb">{selItem.title}</span>
          <button
            className="x"
            aria-label="Close the task"
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

  const ctx: AppCtx = {
    today,
    items,
    byKey,
    settings: cfg,
    plan,
    planItems,
    sched,
    brief,
    briefState,
    githubState,
    clickupState,
    githubWarning: github.warning,
    clickupWarning: clickup.warning,
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
    askItem,
    models,
    setModel: pickModel,
    toggleDone,
    addTask,
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
    startPlan,
    startOver,
    writeBrief,
    toast,
    goPlan,
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
    return parts;
  })();
  const loading = githubState === "loading" || clickupState === "loading";
  const bad = (!["loading", "ok", "off"].includes(githubState)) || (!["loading", "ok", "off"].includes(clickupState));

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
              }}
            >
              ↻
            </button>
          </span>
        </div>
        <div className="body">
          <nav className="rail" aria-label="App">
            {rail("home", ICON.home, "Home", () => (setPage("home"), setSel(null)), page === "home")}
            {rail("plan", ICON.cal, "Day plan", goPlan, onPlan)}
            {rail("chat", ICON.chat, "Conversation", ctx.openMain, page === "home" && mainOpen)}
            {rail("memory", ICON.mem, "What AIfred knows about you", () => (setPage("memory"), setSel(null)), page === "memory")}
            {rail("jobs", ICON.clock, "AIfred's jobs", () => (setPage("jobs"), setSel(null)), page === "jobs")}
            <span className="sp" />
            {rail("settings", ICON.gear, "Settings", () => (setPage("settings"), setSel(null)), page === "settings", update?.state === "ready")}
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
          {onPlan && <PlanView onChat={() => (setPlanChatOpen(true), showSide("chat"))} />}
          {page === "settings" && settings && (
            <div className="scroll page-main page">
              <div className="page-inner">
                <SettingsPage
                  settings={settings}
                  onSaved={setSettings}
                  info={info}
                  clickupToken={clickupToken}
                  onClickupTokenChanged={() => checkClickup(true)}
                  update={update}
                  onUpdateChanged={setUpdate}
                  busy={anyBusy}
                />
              </div>
            </div>
          )}
          {page === "jobs" && (
            <div className="scroll page-main page">
              <div className="page-inner">
                <JobsPage />
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
                />
              </div>
            </div>
          )}

          {selItem && (
            <Detail key={selItem.key} item={selItem} hidden={chatOn && side === "chat"} tabs={tabs} handle={handle} />
          )}
          {page === "home" && mainOpen && (
            <aside className="chat" aria-label="Conversation" hidden={!!selItem && side === "task"}>
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
            <aside className={`chat${planChatOpen ? " open" : ""}`} aria-label="Plan chat" hidden={!!selItem && side === "task"}>
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
