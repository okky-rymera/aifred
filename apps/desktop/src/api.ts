// Typed wrappers around the desktop app's Rust commands.
import { Channel, invoke } from "@tauri-apps/api/core";

export type Turn = { ts: number; role: "user" | "assistant"; text: string; backend: string };

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
};

export function ask(prompt: string, onEvent: (e: ProgressEvent) => void): Promise<Answer> {
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onEvent;
  return invoke<Answer>("ask", { prompt, onEvent: channel });
}

export const history = () => invoke<Turn[]>("history");
/** All facts: the ones in use, then the forgotten ones. */
export const memoryFacts = () => invoke<Fact[]>("memory_facts");
export const addFact = (text: string) => invoke<Fact>("add_fact", { text });
export const editFact = (id: number, text: string) => invoke<Fact>("edit_fact", { id, text });
export const setFactCore = (id: number, core: boolean) => invoke<Fact>("set_fact_core", { id, core });
export const forgetFact = (id: number) => invoke<Fact>("forget_fact", { id });
export const restoreFact = (id: number) => invoke<Fact>("restore_fact", { id });
export const newConversation = () => invoke<void>("new_conversation");
export const tasks = () => invoke<Task[]>("tasks");
/** notes: the optional description. */
export const addTask = (title: string, notes = "") => invoke<Task>("add_task", { task: { title, notes } });
export const setTaskDone = (id: string, done: boolean) => invoke<Task>("set_task_done", { id, done });
export const deleteTask = (id: string) => invoke<void>("delete_task", { id });
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
export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<void>("set_settings", { settings });
export const info = () => invoke<Info>("info");

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
