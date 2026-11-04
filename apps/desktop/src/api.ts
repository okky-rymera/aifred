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
};

export type Info = {
  data_dir: string;
  backend: string;
  github_owners: string | null;
  api_key_set: boolean;
};

export type Task = {
  id: string;
  title: string;
  notes: string;
  done: boolean;
  added_by: "you" | "aifred";
  created: number;
  done_at: number | null;
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
export const getMemory = () => invoke<string>("get_memory");
export const setMemory = (text: string) => invoke<void>("set_memory", { text });
export const newConversation = () => invoke<void>("new_conversation");
export const tasks = () => invoke<Task[]>("tasks");
export const addTask = (title: string) => invoke<Task>("add_task", { title });
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
