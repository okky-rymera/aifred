// Typed wrappers around the desktop app's Rust commands.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** A file that the user sent or keeps on a task (uploads.rs). */
export type FileRef = { id: string; name: string; size: number };

export type Turn = { ts: number; role: "user" | "assistant"; text: string; backend: string; files?: FileRef[] };

/** One message of a side chat: the plan chat or the chat of one item (side.rs). */
export type SideTurn = { id: number; ts: number; role: "user" | "assistant"; text: string; files: FileRef[]; info?: string };

export type ProgressEvent =
  | { kind: "tool_call"; server: string; tool: string; input: Record<string, unknown> }
  | { kind: "notice"; message: string };

export type Answer = {
  text: string;
  stats: { startup_ms: number; total_ms: number; tool_calls: string[] };
  model_ms: number | null;
  cost_usd: number | null;
  /** Facts from memory that the app added to the question, because they match it. */
  memory: string[];
  /** The model you picked for this message. null: the default model. */
  model: string | null;
  /** The short line under the answer: time, model, lookups, and cost. */
  info: string;
};

/** The models that the chat box offers. null is the default model. */
export const CHAT_MODELS: { id: string | null; label: string; hint: string }[] = [
  { id: null, label: "Default", hint: "The Claude Code default model, or AIFRED_MODEL" },
  { id: "fable", label: "Fable", hint: "The most careful model. The slowest." },
  { id: "opus", label: "Opus", hint: "Very careful. Slow." },
  { id: "sonnet", label: "Sonnet", hint: "Careful and faster" },
  { id: "haiku", label: "Haiku", hint: "The fastest. Good for short questions." },
];

export type Info = {
  data_dir: string;
  backend: string;
  github_owners: string | null;
  api_key_set: boolean;
  /** "on", "starting", or "off" with the reason. */
  meaning_search: string;
};

/** One fact that AIfred knows about you. */
export type Fact = {
  id: number;
  text: string;
  /** True: the fact goes into every message. False: AIfred finds it with a search. */
  core: boolean;
  added_by: "you" | "aifred";
  created: number;
  updated: number;
  /** When the fact was forgotten, in Unix seconds. null for a fact in use. */
  forgotten: number | null;
  /** Lowercase topic tags, for example ["testing", "people"]. */
  tags: string[];
};

export type Priority = "urgent" | "high" | "normal" | "low";

export type ChecklistItem = { text: string; done: boolean };

export type Task = {
  id: string;
  title: string;
  notes: string;
  done: boolean;
  added_by: "you" | "aifred";
  created: number;
  done_at: number | null;
  /** The day the task must be done, "YYYY-MM-DD". */
  due: string | null;
  /** The first day to work on the task, "YYYY-MM-DD". Before it, the task waits. */
  start: string | null;
  priority: Priority | null;
  tags: string[];
  checklist: ChecklistItem[];
  files: FileRef[];
};

/** The fields to set. A field that is left out stays as it is. For due,
 * start, and priority, "" clears the field. */
export type TaskChanges = {
  title?: string;
  notes?: string;
  due?: string;
  start?: string;
  priority?: Priority | "";
  tags?: string[];
  checklist?: ChecklistItem[];
  /** The ids of the files of the task. The full new list. */
  files?: string[];
};

export type GithubItem = {
  id: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  reason: "review_requested" | "assigned" | "your_pr";
  updated: string;
  /** For a pull request: the issues it closes. For an issue: its pull requests,
   * also your own pull requests that the list shows inside the issue. */
  links?: GithubLink[];
};

export type GithubLink = {
  id: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  /** True for a pull request, false for an issue. */
  pull_request: boolean;
  /** Set when the linked item is also in your list. */
  reason?: GithubItem["reason"];
};

export type GithubList = {
  /** The open items. AIfred only reads GitHub, so it never changes them. */
  items: GithubItem[];
  /** When the list came from GitHub, in Unix seconds. */
  fetched_at: number;
  /** Set when GitHub could not be asked and this is the older saved list. */
  warning?: string;
};

export type ClickupItem = {
  id: string;
  /** The team's own task id, like "DEV-42", when the workspace uses them. */
  custom_id?: string;
  title: string;
  url: string;
  /** The status name, like "in progress". */
  status: string;
  /** The status color, like "#4194f6". */
  status_color: string;
  /** The list that holds the task. */
  list: string;
  workspace: string;
  priority?: "urgent" | "high" | "normal" | "low";
  /** Unix milliseconds. */
  due?: number;
  /** Unix milliseconds. */
  updated: number;
};

export type ClickupList = {
  /** Open tasks assigned to you, newest change first. */
  items: ClickupItem[];
  /** When the list came from ClickUp, in Unix seconds. */
  fetched_at: number;
  /** Set when ClickUp could not be asked and this is the older saved list. */
  warning?: string;
};

export type ClickupTokenStatus = {
  set: boolean;
  /** True when AIFRED_CLICKUP_TOKEN gives the token. The window cannot change it. */
  from_env: boolean;
};

export type Settings = {
  /** How often the GitHub and ClickUp lists reload. 0 means only when you click reload. */
  refresh_minutes: number;
  /** The work day, "HH:MM". */
  work_start: string;
  work_end: string;
  /** When lunch starts, "HH:MM", and how long it is. 0: no lunch. */
  lunch_start: string;
  lunch_minutes: number;
  /** AIfred writes the brief by itself once a day. */
  auto_brief: boolean;
};

export const DEFAULT_SETTINGS: Settings = {
  refresh_minutes: 15,
  work_start: "09:00",
  work_end: "17:30",
  lunch_start: "12:30",
  lunch_minutes: 45,
  auto_brief: true,
};

/** One item of a day plan. `key` names the item (items.rs). */
export type PlanItem = { key: string; why?: string; goal?: number; /** A fixed start time, in minutes after midnight. */ at?: number };
/** `mode` "review": a review put items in a day with no plan. You did not pick yet how to plan the day. */
export type DayPlan = { day: string; mode: "ai" | "self" | "review"; items: PlanItem[]; updated: number };
export type Estimate = { minutes: number; by: "you" | "aifred" };
/** The text that AIfred can draft for a step later: an email, text in a Google Doc, or a change in a Google Sheet. */
export type DraftKind = "email" | "doc" | "sheet";

export type Brief = {
  day: string;
  written: number;
  text: string;
  why: Record<string, string>;
  /** The id of the goal that an item serves, by item key. */
  goal?: Record<string, number>;
  /** The next action for each item in `why`. */
  next?: Record<string, string>;
  /** The next actions that AIfred can draft later. A next action that is not here is for you. */
  draft?: Record<string, DraftKind>;
  /** Today's missions, the most important first (missions.rs). */
  missions?: Mission[];
  /** AIfred's moves of items into subtasks since the brief of the day
   * before, that are still in place (sort.rs). */
  moves?: Moved[];
  /** The count of older moves that `moves` does not give: it gives only the newest. */
  moves_more?: number;
  /** The items that AIfred has questions about, at most 5 (context::MAX_ASK). "Help AIfred understand" asks about them. */
  ask?: string[];
  /** The question about the items in `ask` (context::QUESTION in Rust). */
  question?: string;
  /** AIfred's open questions about each item in `ask`, the next one first.
   * An item with no list gets `question`. */
  questions?: Record<string, string[]>;
  /** True: the brief has no missions until you answer the questions in
   * `ask`. With no questions left, the app writes the brief again. */
  held?: boolean;
  /** The title of each item in `ask`, also when its list did not load. */
  titles?: Record<string, string>;
};

/** One move of AIfred: `key` is now a subtask of `main`, for the reason `why`. */
export type Moved = { id: number; ts: number; key: string; title: string; main: string; main_title: string; why: string };

/** One item of a mission. `open` is false when the item is done or closed. */
export type MissionItem = {
  key: string;
  title: string;
  open: boolean;
  /** Who put the item in the mission. The brief does not move an item that you added. */
  by: "you" | "aifred";
};
/** How far a mission is: (steps_done + items_closed) / (steps + items). */
export type MissionProgress = { steps_done: number; steps: number; items_closed: number; items: number; percent: number };
/** One unit of work that the brief made from one or more items. The first
 * item is the item of the next action: the mission shows its reason, its
 * next action, and its goal. */
export type Mission = {
  id: number;
  title: string;
  doer: "you" | "aifred";
  day: string | null;
  rank: number | null;
  status: "open" | "done";
  items: MissionItem[];
  checklist: ChecklistItem[];
  progress: MissionProgress;
};

/** One of the user's goals (goals.rs). */
export type Goal = { id: number; text: string; created: number; updated: number };
/** `stale`: the user has not changed or checked the goals for 30 days. */
export type Goals = { goals: Goal[]; checked: number; stale: boolean };
/** At most this many goals (goals.rs). */
export const MAX_GOALS = 5;
export const goals = () => invoke<Goals>("goals");
export const addGoal = (text: string) => invoke<Goals>("add_goal", { text });
export const editGoal = (id: number, text: string) => invoke<Goals>("edit_goal", { id, text });
export const removeGoal = (id: number) => invoke<Goals>("remove_goal", { id });
/** The user says that the goals are still right. */
export const confirmGoals = () => invoke<Goals>("confirm_goals");
export type ItemMeta = { key: string; first_seen: number; new: boolean; summary?: string };
export type ItemDetails = {
  key: string;
  details: string;
  /** The custom fields of a ClickUp task that have a value. */
  fields?: { name: string; value: string }[];
  summary?: string;
};

/** The model of one message: a model id, "default", or undefined for the last pick. */
export type ModelPick = string | undefined;

/** A question in the main conversation. `files`: ids from uploadFile. */
export function ask(prompt: string, onEvent: (e: ProgressEvent) => void, opts: { model?: ModelPick; files?: string[] } = {}): Promise<Answer> {
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onEvent;
  return invoke<Answer>("ask", { prompt, onEvent: channel, model: opts.model, files: opts.files ?? [] });
}

/** A question in a side chat: "plan:YYYY-MM-DD" or "item:<key>". */
export function askSide(
  thread: string,
  prompt: string,
  onEvent: (e: ProgressEvent) => void,
  opts: { model?: ModelPick; files?: string[] } = {},
): Promise<Answer> {
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onEvent;
  return invoke<Answer>("ask_side", { thread, prompt, onEvent: channel, model: opts.model, files: opts.files ?? [] });
}

export const sideHistory = (thread: string) => invoke<SideTurn[]>("side_history", { thread });
export const clearSide = (thread: string) => invoke<void>("clear_side", { thread });

/** Saves one file. The bytes go as the body, so a large file needs no JSON. */
export async function uploadFile(file: File): Promise<FileRef> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<FileRef>("upload_file", bytes, { headers: { "x-file-name": encodeURIComponent(file.name) } });
}
export const openFile = (id: string) => invoke<void>("open_file", { id });

export const dayPlan = (day: string) => invoke<DayPlan | null>("day_plan", { day });
export const setDayPlan = (day: string, mode: DayPlan["mode"], items: PlanItem[]) => invoke<DayPlan>("set_day_plan", { day, mode, items });
/** The plans of the days from `from` to `to`, both included. A day with no plan is not in the list. */
export const dayPlans = (from: string, to: string) => invoke<DayPlan[]>("day_plans", { from, to });
/** Moves one item to the plan of another day. Returns the plan of `from`, then the plan of `to`. */
export const movePlanItem = (from: string, to: string, key: string, at: number | null, index: number | null) =>
  invoke<[DayPlan | null, DayPlan]>("move_plan_item", { from, to, key, at, index });
export const clearDayPlan = (day: string) => invoke<void>("clear_day_plan", { day });
export const estimates = () => invoke<Record<string, Estimate>>("estimates");
/** Your own time guess. null removes it. */
export const setEstimate = (key: string, minutes: number | null) => invoke<void>("set_estimate", { key, minutes });
export const brief = (day: string) => invoke<Brief | null>("brief", { day });
/** One AI call. */
/** `release` true: the brief of `day` stops holding its missions for the questions. */
export const writeBrief = (day: string, release = false) => invoke<Brief>("write_brief", { day, release });
export const itemMeta = () => invoke<ItemMeta[]>("item_meta");

/** "not_today": the item does not need you today. "wrong_priority": the brief ranked it too high. */
export type CorrectionKind = "not_today" | "wrong_priority";
export type Correction = { id: number; key: string; kind: CorrectionKind; day: string; title: string; place: string; ts: number };
/** `learned`: the memory fact that AIfred saved from this and earlier corrections. */
export type Corrected = { correction: Correction; learned?: Fact };
/** Ticks step `index` of a mission, or takes the tick away. The items do not change. */
export const setMissionStep = (id: number, index: number, done: boolean) => invoke<Mission>("set_mission_step", { id, index, done });
/** The mission is done. Close the items that you picked with their own commands. */
export const finishMission = (id: number) => invoke<null>("finish_mission", { id });
/** The undo of finishMission: the mission is open again, with its checklist. */
export const reopenMission = (id: number) => invoke<null>("reopen_mission", { id });
/** Where an item was before you added it to a mission or took it out (missions.rs). */
export type MissionBefore = {
  key: string;
  mission: number;
  spot: { mission: number; pos: number; by: string } | null;
  out: boolean;
};
export type MissionChanged = { mission: Mission; before: MissionBefore };
/** Adds an open item to a mission. It leaves its other mission. The brief does not take it away. */
export const addMissionItem = (id: number, key: string) => invoke<MissionChanged>("add_mission_item", { id, key });
/** Takes an item out of a mission. The brief does not put it back. The last open item cannot go out. */
export const removeMissionItem = (id: number, key: string) => invoke<MissionChanged>("remove_mission_item", { id, key });
/** The undo of addMissionItem and removeMissionItem. */
export const restoreMissionItem = (before: MissionBefore) => invoke<null>("restore_mission_item", { before });
/** A correction of each open item of the mission, all saved or none. A mission with no open item is an error. */
export const correctMission = (id: number, kind: CorrectionKind, day: string) => invoke<Corrected[]>("correct_mission", { id, kind, day });
/** Your correction of the brief of `day` (learn.rs). The brief stops showing a reason for the item that day. */
export const correctBrief = (key: string, kind: CorrectionKind, day: string) => invoke<Corrected>("correct_brief", { key, kind, day });
/** Removes a correction. A fact that it made is forgotten. */
export const undoCorrection = (id: number) => invoke<void>("undo_correction", { id });

/** The item that you work on now (worktime.rs). `before`: seconds from earlier sessions of the item. */
export type Running = { key: string; title: string; started: number; before: number };
/** An item in the end-of-day check (review.rs). `minutes`: the real time of a finished item. */
export type DayItem = { key: string; title: string; place: string; minutes?: number };
/** The end-of-day check of one day. `left`: still open, from the plan and the tasks due that day. */
export type DayCheck = { day: string; done: DayItem[]; left: DayItem[]; finished: boolean; moved: string[] };
/** An open item with no change for 3 weeks. `last`: Unix seconds. */
export type StaleItem = { key: string; title: string; place: string; last: number; days: number };
/** A goal and the finished items that served it in the last 7 days. */
export type GoalWeek = { id: number; text: string; done: number };
export type WeekReview = { due: boolean; last: string | null; items: StaleItem[]; more: number; goals: GoalWeek[]; goals_stale: boolean };
export type PickKind = "do" | "plan" | "delegate" | "drop";
/** A pick in the weekly review. `note`: the day for "plan", the person for "delegate". */
export type ReviewPick = { id: number; key: string; pick: PickKind; title: string; place: string; note: string; ts: number };
/** The weekly review comes this many days after the last one (review.rs). */
export const WEEK_DAYS = 7;
export const dayCheck = (day: string) => invoke<DayCheck>("day_check", { day });
/** The items in `moved` go into the plan of the next day. */
export const finishDay = (day: string, moved: string[]) => invoke<DayCheck>("finish_day", { day, moved });
export const weekReview = (today: string) => invoke<WeekReview>("week_review", { today });
export const finishWeekReview = (today: string) => invoke<WeekReview>("finish_week_review", { today });
export const reviewPick = (key: string, pick: PickKind, note: string, today: string) => invoke<ReviewPick>("review_pick", { key, pick, note, today });
export const undoReviewPick = (id: number) => invoke<void>("undo_review_pick", { id });

/** A finished item: the real time next to the time guess, in minutes.
 * `minutes` 0: only forgotten timers. `skipped`: the timers of more than 10 hours, which do not count. */
export type WorkDone = { key: string; title: string; minutes: number; guess: number | null; skipped: number };
export const workRunning = () => invoke<Running | null>("work_running");
/** Starts work on an item. The work on another item stops. */
export const startWork = (key: string) => invoke<Running>("start_work", { key });
export const stopWork = () => invoke<void>("stop_work");
/** The item is finished. null: it has no work time. */
export const finishWork = (key: string) => invoke<WorkDone | null>("finish_work", { key });
/** The item is open again: its real time does not count. */
export const unfinishWork = (key: string) => invoke<void>("unfinish_work", { key });

/** When a job runs. Times are local, "YYYY-MM-DDTHH:MM". */
export type Trigger =
  | { kind: "once"; at: string }
  | { kind: "repeat"; start: string; rule: string }
  | { kind: "event"; event: string; wait_secs: number };

/** One of AIfred's own jobs: a reminder, an AI run, the brief update, or a rule that adds your task at each time. */
export type Job = {
  id: string;
  title: string;
  action: "remind" | "agent" | "brief" | "task";
  /** The reminder text, the instructions for the AI, or the notes of each new task. */
  message: string;
  trigger: Trigger;
  tz: string;
  status: "active" | "paused" | "done" | "off";
  /** Unix seconds, or null. */
  next_run: number | null;
  created_by: string;
  created: number;
  updated: number;
  /** Failed runs in a row. */
  failures: number;
  /** The trigger in plain words. */
  when: string;
  /** True when Start can work: the job is not active, and it has a time to run. */
  can_start: boolean;
};

export type JobRun = {
  id: number;
  job: string;
  planned: number;
  started: number | null;
  finished: number | null;
  state: "running" | "ok" | "quiet" | "skipped" | "failed";
  late: boolean;
  note: string;
  output: string;
  cost_usd: number | null;
};

/** all false: only the active and paused jobs. */
export const jobs = (all: boolean) => invoke<Job[]>("jobs", { all });
/** The job that writes the brief again when you finish a task. */
export const briefJob = () => invoke<Job>("brief_job");
/** Newest first. id null: the runs of all jobs. */
/** `brief` false: the runs of the brief job (action "brief") are left out. */
export const jobRuns = (id: string | null, limit: number, skipped: boolean, brief: boolean) =>
  invoke<JobRun[]>("job_runs", { id, limit, skipped, brief });
export const setJobStatus = (id: string, status: "active" | "paused") => invoke<Job>("set_job_status", { id, status });
export const deleteJob = (id: string) => invoke<void>("delete_job", { id });
export const itemDetails = (key: string) => invoke<ItemDetails>("item_details", { key });

/** AIfred's short notes about one item, from your answers or from AIfred's own look-up (context.rs). */
/** `by`: "lookup" when AIfred wrote the notes from the item text before the brief. */
export type ItemContext = { key: string; text: string; by: "you" | "aifred" | "lookup"; updated: number };
/** Loads AIfred's notes about one item. null: the item has no notes. */
export const itemContext = (key: string) => invoke<ItemContext | null>("item_context", { key });
/** Saves your notes. "" clears them. It gives the notes from before, for the Undo. */
export const setItemContext = (key: string, text: string) => invoke<ItemContext | null>("set_item_context", { key, text });
export const restoreItemContext = (key: string, before: ItemContext | null) => invoke<null>("restore_item_context", { key, before });
/** AIfred does not ask about the item again. `undo` true asks again. */
export const skipContextQuestion = (key: string, undo = false) => invoke<null>("skip_context_question", { key, undo });
/** One step that AIfred suggests for an item. `draft`: AIfred can draft its text later. */
export type Step = { text: string; draft?: DraftKind };
/** AIfred splits an item into steps (steps.rs). One AI call. It saves nothing. */
export const suggestSteps = (key: string) => invoke<Step[]>("suggest_steps", { key });
/** What addSteps did. `created`: AIfred made a new task under the item. */
export type AddedSteps = { task: Task; added: number; created: boolean };
/** Puts the steps on the open task of the item's group, or on a new task under the item. */
export const addSteps = (key: string, steps: string[]) => invoke<AddedSteps>("add_steps", { key, steps });
/** A reply to an email that AIfred wrote (drafts.rs), for you to check. `to` comes from the email, not from the AI.
 * `can_save`: false until you turn on Gmail drafts in Settings and connect Google again. */
export type ReplyDraft = { key: string; to: string; to_name: string; subject: string; body: string; last_message: string; can_save: boolean };
/** AIfred writes a reply to an email item. One AI call with no tools. It saves nothing. */
export const writeReply = (key: string, note: string) => invoke<ReplyDraft>("write_reply", { key, note });
/** One action of AIfred in another app (actions.rs). `undone_at`: Undo ran. `note`: why Undo did not remove the change. */
export type Action = { id: number; at: number; app: "gmail" | "docs" | "sheets"; kind: "draft" | "suggestion" | "comment"; item: string; title: string; after: string; url: string; undone_at: number | null; note: string };
/** Saves the reply that you checked as a draft in its Gmail thread. It never sends. */
export const saveReplyDraft = (d: ReplyDraft) => invoke<Action>("save_reply_draft", { key: d.key, to: d.to, lastMessage: d.last_message, body: d.body });
/** One change in a Google Doc: `find` is text in the Doc, `replace` its new text. */
export type DocEdit = { find: string; replace: string; why: string };
/** Changes to a Google Doc that AIfred wrote (docs.rs), for you to check. `revision`: the save fails when the Doc changed.
 * `tab`: the tab of the Doc with the changes (empty: a Doc with no tabs). */
export type DocProposal = { key: string; doc: string; tab: string; title: string; url: string; revision: string; can_edit: boolean; edits: DocEdit[]; message: string };
/** The first Google Doc link and the first Google Sheet link of an item: in its description, or in another item of the same work. */
export type WorkLinks = { doc: string | null; sheet: string | null };
// The Doc part and the Sheet part ask at the same time: they share one call.
// The call is forgotten when it ends, so the next ask reads the items again.
const asking = new Map<string, Promise<WorkLinks>>();
const workLinks = (key: string) => {
  let p = asking.get(key);
  if (!p) {
    p = invoke<WorkLinks>("work_links", { key }).finally(() => asking.delete(key));
    asking.set(key, p);
  }
  return p;
};
/** The first Google Doc link in the item's description, or in another item of the same work. */
export const docLink = (key: string) => workLinks(key).then((l) => l.doc);
/** Reads the Doc and suggests changes in one AI call. It saves nothing. */
export const suggestDocChanges = (key: string, link: string, note: string) => invoke<DocProposal>("suggest_doc_changes", { key, link, note });
/** Saves the changes that you checked as suggestions in the Doc. It never edits the Doc directly. */
export const saveDocSuggestions = (p: DocProposal, edits: DocEdit[]) => invoke<Action>("save_doc_suggestions", { key: p.key, doc: p.doc, tab: p.tab, revision: p.revision, edits });
/** One comment on a cell of a Google Sheet. `value`: the value of the cell when AIfred read it. */
export type SheetComment = { cell: string; text: string; value: string };
/** Comments on a Google Sheet that AIfred wrote (sheets.rs), for you to check. `gid`: the tab. `check`: a hash of the tab as AIfred read it; the save fails when the tab changed. */
export type SheetProposal = { key: string; sheet: string; gid: number; tab: string; title: string; url: string; comments: SheetComment[]; message: string; check: string };
/** The first Google Sheet link in the item's description, or in another item of the same work. */
export const sheetLink = (key: string) => workLinks(key).then((l) => l.sheet);
/** Reads the Sheet and suggests comments in one AI call. It saves nothing. */
export const suggestSheetComments = (key: string, link: string, note: string) => invoke<SheetProposal>("suggest_sheet_comments", { key, link, note });
/** Saves the comments that you checked on the cells of the Sheet. It never changes a cell. */
export const saveSheetComments = (p: SheetProposal, comments: SheetComment[]) => invoke<Action>("save_sheet_comments", { key: p.key, sheet: p.sheet, gid: p.gid, check: p.check, comments });
/** AIfred's actions, newest first. With `item`: only the actions of that item. */
export const actions = (item?: string) => invoke<Action[]>("actions", { item: item ?? null });
/** Undoes an action: deletes its Gmail draft, when nobody changed it in Gmail, its Doc suggestions that are still open, or its Sheet comments that nobody resolved or answered. */
export const undoAction = (id: number) => invoke<Action>("undo_action", { id });

export const history = () => invoke<Turn[]>("history");
/** All facts: the ones in use, then the forgotten ones. */
export const memoryFacts = () => invoke<Fact[]>("memory_facts");
export const addFact = (text: string) => invoke<Fact>("add_fact", { text });
export const editFact = (id: number, text: string) => invoke<Fact>("edit_fact", { id, text });
export const setFactCore = (id: number, core: boolean) => invoke<Fact>("set_fact_core", { id, core });
export const setFactTags = (id: number, tags: string[]) => invoke<Fact>("set_fact_tags", { id, tags });
export const forgetFact = (id: number) => invoke<Fact>("forget_fact", { id });
export const restoreFact = (id: number) => invoke<Fact>("restore_fact", { id });
/** Deletes a forgotten fact for good. false: deleted, but the database file is not cleaned yet. */
export const deleteFact = (id: number) => invoke<boolean>("delete_fact", { id });
export const newConversation = () => invoke<void>("new_conversation");
export const tasks = () => invoke<Task[]>("tasks");
/** notes: the optional description. */
export const addTask = (title: string, notes = "") => invoke<Task>("add_task", { task: { title, notes } });
/** Adds a task with all the fields in `fields`. The title is necessary. */
export const createTask = (fields: TaskChanges & { title: string }) => invoke<Task>("add_task", { task: fields });
/** `work`: the real time that AIfred kept when the task is done now. null: no work time, or the task is open again. */
export type TaskDone = { task: Task; work: WorkDone | null };
/** The backend ends the work of a task that is done, and keeps its real time (tasks.rs). */
export const setTaskDone = (id: string, done: boolean) => invoke<TaskDone>("set_task_done", { id, done });
export const deleteTask = (id: string) => invoke<void>("delete_task", { id });
/** One event of the user's Google Calendar (calendar.rs). Times in Unix seconds. */
export type CalEvent = {
  id: string;
  title: string;
  start: number;
  end: number;
  all_day: boolean;
  /** It takes the user's time: the plan goes around it. */
  busy: boolean;
  attendees: string[];
  more_attendees: number;
  response: string | null;
  url: string;
  description: string;
};
export type CalendarList = { fetched_at: number; events: CalEvent[]; warning?: string };
/** null: Google is not connected, so there is no calendar. */
export const calendarEvents = (force: boolean) => invoke<CalendarList | null>("calendar_events", { force });
/** One email thread (gmail.rs). Times in Unix seconds. */
export type Mail = {
  id: string;
  subject: string;
  /** "reply": you must answer. "sent": you wait for an answer. */
  kind: "reply" | "sent";
  /** "reply": who wrote the last message. "sent": who you wrote to. */
  people: string[];
  more_people: number;
  email: string;
  /** The email addresses of all people. */
  emails?: string[];
  /** "reply": who a reply goes to (Reply-To, else the sender). Empty in emails saved before AIfred read it. */
  reply_to?: string;
  last_at: number;
  /** "reply": the first message after your last one. "sent": your last message. */
  since: number;
  snippet: string;
  /** "reply": you are in To, not only in Cc. */
  direct: boolean;
  unread: boolean;
  question: boolean;
  messages: number;
  url: string;
};
export type MailList = { fetched_at: number; reply: Mail[]; sent: Mail[]; warning?: string };
/** null: Gmail is not connected, or the Google login does not allow it. */
export const gmailThreads = (force: boolean) => invoke<MailList | null>("gmail_threads", { force });
/** The saved emails at any age. It never downloads. */
export const gmailSaved = () => invoke<MailList | null>("gmail_saved");
/** Hides an email until a new message comes in it. hide false: show it again (Undo). */
export const hideEmail = (id: string, hide: boolean) => invoke<void>("hide_email", { id, hide });
/** One item of the "waiting for" list (waiting.rs). Days are "YYYY-MM-DD". */
export type Waiting = {
  id: number;
  what: string;
  who: string;
  since: string;
  follow_up: string | null;
  told: boolean;
  /** The Gmail thread, when the item came from an email. */
  thread: string | null;
  url: string | null;
  done: boolean;
  /** Why it closed, for example "Ana Lee answered on Thu 5 Nov". */
  done_why: string;
  created: number;
  updated: number;
};
export type NewWaiting = { what?: string; who?: string; since?: string; follow_up?: string; thread?: string };
/** Only the fields that are set change. follow_up "": no follow-up day. */
export type WaitingChanges = { what?: string; who?: string; follow_up?: string; done?: boolean };
/** One item under its parent item, in AIfred only (links.rs). `by`: who placed
 * it, "you" or "aifred". `why`: AIfred's reason, empty when you placed it. */
export type ItemLink = { key: string; main: string; by?: "you" | "aifred"; why?: string };
export type Links = { links: ItemLink[] };
export const itemLinks = () => invoke<Links>("item_links");
/** How things were before a move by the user (links.rs Before): the old
 * link of the item, and the sort marks of the item and its parents. The
 * Undo sends it back. */
export type LinkBefore = { key: string; link: ItemLink | null; marks: { key: string; fixed: boolean | null }[] };
/** The links after a move by the user, and how things were before it. */
export type LinkChanged = Links & { before: LinkBefore };
/** Puts `key` under `main` as a subtask, with its own subtasks. */
export const linkItems = (main: string, key: string) => invoke<LinkChanged>("link_items", { main, key });
/** The Undo of a move by the user: the item and the sort marks go back to how they were. */
export const restoreLink = (before: LinkBefore) => invoke<Links>("restore_link", { before });
/** Moves `key` out from under its parent, with its own subtasks. */
export const unlinkItem = (key: string) => invoke<LinkChanged>("unlink_item", { key });
/** Undoes AIfred's moves (sort.rs): each item goes back to where it was. AIfred does not move these items again. */
export const undoMoves = (ids: number[]) => invoke<Links>("undo_moves", { ids });

/** One item that waits until another item is finished, in AIfred only
 * (blockers.rs). `by`: who set it ("chat": the AI, because you asked in a chat). `why`: AIfred's reason. */
export type ItemBlocker = { key: string; blocker: string; by: "you" | "chat" | "aifred"; why: string };
export type Blockers = { blockers: ItemBlocker[] };
export const itemBlockers = () => invoke<Blockers>("item_blockers");
/** How the link was before a change, for its Undo (blockers.rs Before). `at` and `gone`: how the change left the link. */
export type BlockerBefore = { key: string; blocker: string; row: { by: string; why: string; removed: boolean } | null; at: number; gone: boolean };
export type BlockerChanged = Blockers & { before: BlockerBefore };
/** Says that `key` waits for `blocker`. */
export const addBlocker = (key: string, blocker: string) => invoke<BlockerChanged>("add_blocker", { key, blocker });
/** Takes away the link. AIfred does not add it again by itself. */
export const removeBlocker = (key: string, blocker: string) => invoke<BlockerChanged>("remove_blocker", { key, blocker });
/** The Undo of addBlocker or removeBlocker. */
export const restoreBlocker = (before: BlockerBefore) => invoke<Blockers>("restore_blocker", { before });

/** The open items, then the items that closed in the last 3 days. */
export const waitingItems = () => invoke<Waiting[]>("waiting_items");
export const addWaiting = (item: NewWaiting) => invoke<Waiting>("add_waiting", { item });
export const updateWaiting = (id: number, changes: WaitingChanges) => invoke<Waiting>("update_waiting", { id, changes });
export const deleteWaiting = (id: number) => invoke<void>("delete_waiting", { id });
/** The Google connection (google.rs). `features`: each feature, and true when the login allows it. */
export type GoogleStatus = {
  client_id: string;
  secret_set: boolean;
  connected: boolean;
  email: string | null;
  features: [string, boolean][];
  /** The features that the user can turn off. */
  optional: string[];
  /** The features that the user turned off. The login does not ask for them. */
  off: string[];
};
/** Turns an optional Google feature (Gmail, Gmail drafts) on or off. */
export const setGoogleFeature = (name: string, on: boolean) => invoke<GoogleStatus>("set_google_feature", { name, on });
export const googleStatus = () => invoke<GoogleStatus>("google_status");
export const setGoogleClient = (clientId: string, clientSecret: string) => invoke<GoogleStatus>("set_google_client", { clientId, clientSecret });
/** Opens Google's login page and waits until the user is back. */
export const connectGoogle = () => invoke<GoogleStatus>("connect_google");
export const disconnectGoogle = () => invoke<GoogleStatus>("disconnect_google");
/** force: ask GitHub now, and do not use the saved list. */
export const githubItems = (force: boolean) => invoke<GithubList>("github_items", { force });
/** force: ask ClickUp now, and do not use the saved list. */
export const clickupItems = (force: boolean) => invoke<ClickupList>("clickup_items", { force });
/** Closes the task in ClickUp. Returns the status it had before, for the undo. */
export const clickupMarkDone = (id: string) => invoke<string>("clickup_mark_done", { id });
export const clickupUndoDone = (id: string, status: string) => invoke<void>("clickup_undo_done", { id, status });
export const clickupTokenStatus = () => invoke<ClickupTokenStatus>("clickup_token_status");
/** Checks the token with ClickUp and saves it. Returns your ClickUp user name. */
export const setClickupToken = (token: string) => invoke<string>("set_clickup_token", { token });
export const removeClickupToken = () => invoke<void>("remove_clickup_token");
/** The model you picked last in the chat box. null: the default model. */
export const chatModel = () => invoke<string | null>("chat_model");
export const setChatModel = (model: string | null) => invoke<void>("set_chat_model", { model });
/** Changes the given fields of a local task. */
export const editTask = (id: string, changes: TaskChanges) => invoke<Task>("edit_task", { id, changes });
/** Ticks or un-ticks one step. Only this step changes. `text` is the step's
 * text: the tick fails when the checklist changed after the list loaded. */
export const setTaskStep = (id: string, index: number, text: string, done: boolean) =>
  invoke<Task>("set_task_step", { id, index, text, done });
/** The Undo of addSteps: takes out the steps with these texts that are not done, in one save. */
export const removeTaskSteps = (id: string, steps: string[]) => invoke<Task>("remove_task_steps", { id, steps });
export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<void>("set_settings", { settings });
export const info = () => invoke<Info>("info");

/** What the app knows about updates (main.rs). */
export type UpdateStatus = {
  /** The version of this program. */
  current: string;
  state: "off" | "idle" | "checking" | "none" | "downloading" | "ready" | "error";
  /** The new version, when one is found. */
  latest: string | null;
  /** The release page of the new version. */
  page: string | null;
  /** Why updates are off, or the error. */
  message: string | null;
  /** The last check, in Unix seconds. */
  checked_at: number | null;
};

export const updateStatus = () => invoke<UpdateStatus>("update_status");
/** Checks GitHub now. When there is a new version, it also downloads it. */
export const checkUpdate = () => invoke<UpdateStatus>("check_update");
/** Installs the downloaded version and starts it. This window closes. */
export const restartToUpdate = () => invoke<void>("restart_to_update");

/** Plain-language label for a tool call, shown while AIfred works. */
export function describe(e: ProgressEvent): string {
  if (e.kind === "notice") return e.message;
  const i = e.input as { repo?: string; number?: number };
  switch (e.tool) {
    case "my_issues":
      return "Checking issues assigned to you";
    case "review_requests":
      return "Checking reviews you were asked to do";
    case "my_pull_requests":
      return "Checking your open pull requests";
    case "list_tasks":
      return "Checking your to-do list";
    case "add_task":
      return `Adding task: ${(e.input as { title?: string }).title ?? ""}`;
    case "update_task":
      return "Changing a task";
    case "complete_task":
      return "Marking a task done";
    case "overview":
      return "Reading all your open work";
    case "day_plan":
      return "Reading your day plan";
    case "set_day_plan":
      return "Saving your day plan";
    case "move_to_day":
      return "Moving an item to another day";
    case "my_tasks":
      return "Checking your ClickUp tasks";
    case "task_details":
      return `Reading ClickUp task ${(e.input as { id?: string }).id ?? ""}`.trim();
    case "item_details":
      return `Reading ${i.repo ?? "item"}#${i.number ?? "?"}`;
    default:
      return `Using ${e.server} → ${e.tool}`;
  }
}

/** The event that AIfred sends when its scheduler adds a task (a repeating task). */
export const TASKS_CHANGED = "tasks-changed";
/** Calls `f` each time AIfred adds a task by itself. Returns the stop function. */
export const onTasksChanged = (f: () => void) => listen(TASKS_CHANGED, () => f());
/** The event that AIfred sends when its sort moved new items into subtasks (sort.rs). */
export const ITEMS_SORTED = "items-sorted";
/** Calls `f` each time AIfred's sort moved items. Returns the stop function. */
export const onItemsSorted = (f: () => void) => listen(ITEMS_SORTED, () => f());
