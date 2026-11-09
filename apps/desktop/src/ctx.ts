// The data and the actions that all screens share. App.tsx makes them.
import { createContext, useContext } from "react";
import type * as api from "./api";
import type { ChatMsg } from "./Chat";
import type { DayEvents, Item, Schedule } from "./items";

export type CardMode = "auto" | "open" | "hidden";

export type ListState = "loading" | "ok" | "off" | string;

/** A chat: its messages, and the work in progress. */
export type ChatState = { msgs: ChatMsg[]; busy: boolean; progress: string[] };

/** The question talk on the home page: the item, and AIfred's question
 * that started it. */
export type Talk = { key: string; title: string; question: string };

export type AppCtx = {
  today: string;
  items: Item[];
  byKey: Map<string, Item>;
  settings: api.Settings;
  plan: api.DayPlan | null;
  /** The plan items that are still open, in order. */
  planItems: Item[];
  sched: Schedule;
  /** The plans of today and the next days, by day. null: the day has no plan. */
  plans: Record<string, api.DayPlan | null>;
  /** The open plan items, the schedule, and the calendar events of today and each next day, by day. */
  week: Record<string, { items: Item[]; sched: Schedule; events: DayEvents }>;
  /** The events of the user's Google Calendar, today and the next days. Empty when it is not connected. */
  events: api.CalEvent[];
  /** The day on the Calendar page. pickDay() also shows the plan chat of that day. */
  calDay: string;
  pickDay: (day: string) => void;
  calView: "day" | "week";
  setCalView: (v: "day" | "week") => void;
  brief: api.Brief | null;
  /** Reads the brief of `day` again, when it is still on the screen. */
  reloadBrief: (day: string | undefined) => void;
  /** null until the goals are loaded. */
  goals: api.Goals | null;
  /** The text of a goal by its id. */
  goalText: (id: number | undefined) => string | undefined;
  /** Runs one goal change and shows the new goals. False when it failed. */
  changeGoals: (change: () => Promise<api.Goals>) => Promise<boolean>;
  /** "idle", "writing", or the error of the last try. */
  briefState: string;
  /** True until today's brief and the lists that its questions need are loaded. */
  homeLoading: boolean;
  githubState: ListState;
  clickupState: ListState;
  githubWarning?: string;
  clickupWarning?: string;
  /** "off": Gmail is not connected, or the Google login does not allow it. */
  mailState: ListState;
  mailWarning?: string;
  /** Emails that you sent and that got no answer yet: suggestions for the "waiting for" list. */
  sentMail: api.Mail[];
  /** The "waiting for" list: the open items, then the items that closed in the last 3 days. */
  waiting: api.Waiting[];
  waitingError: string;
  /** The subtasks: each item under its parent (links.rs). */
  links: api.Links;
  /** Puts `key` under `main` as a subtask, with an undo. */
  linkItems: (main: string, key: string) => Promise<boolean>;
  /** Moves an item out from under its parent, with an undo. */
  unlinkItem: (key: string) => Promise<boolean>;
  /** Undoes AIfred's moves from the brief: the items go back to where they were. */
  undoMoves: (moves: api.Moved[]) => Promise<boolean>;
  trackEmail: (m: api.Mail, followUp?: string) => Promise<boolean>;
  hideSent: (m: api.Mail) => Promise<boolean>;
  addWaiting: (item: api.NewWaiting) => Promise<boolean>;
  changeWaiting: (id: number, changes: api.WaitingChanges) => Promise<boolean>;
  removeWaiting: (w: api.Waiting) => Promise<boolean>;
  tasksError: string;

  sel: string | null;
  open: (key: string) => void;
  close: () => void;
  /** The id of the mission that is open on the right side, or null. */
  selMission: number | null;
  /** Opens a mission of today on the right side. */
  openMission: (id: number) => void;

  /** The main conversation. */
  main: ChatState;
  mainOpen: boolean;
  openMain: () => void;
  sendMain: (text: string, files: api.FileRef[]) => Promise<boolean>;
  planChat: ChatState;
  sendPlan: (text: string, files?: api.FileRef[], day?: string) => Promise<boolean>;
  /** The chat of an item, by its key, or of a mission, by missionChatKey(). */
  itemChat: (key: string) => ChatState;
  askItem: (key: string, text: string, files: api.FileRef[]) => Promise<boolean>;
  /** Loads the old messages of an item or mission chat, one time. */
  loadItemChat: (key: string) => void;

  /** The model of each chat box. null: the default model. */
  models: { main: string | null; plan: string | null; item: string | null };
  setModel: (box: "main" | "plan" | "item", m: string | null) => void;

  /** True when the item closed. */
  toggleDone: (i: Item) => Promise<boolean>;
  /** The item that you work on now, or null. */
  work: api.Running | null;
  /** Starts work on an item. The work on another item stops. */
  startWork: (key: string) => void;
  stopWork: () => void;
  /** The "Done" button of the work timer, for the item `key`. */
  finishItem: (key: string) => void;
  /** You say that the brief is wrong about an item. */
  correctBrief: (i: Item, kind: api.CorrectionKind) => void;
  /** Ticks step `index` of a mission, or takes the tick away. */
  missionStep: (m: api.Mission, index: number) => void;
  /** You say that the brief is wrong about a mission. */
  correctMission: (m: api.Mission, kind: api.CorrectionKind) => void;
  /** Adds an open item to a mission, with an undo. */
  addToMission: (m: api.Mission, key: string) => Promise<boolean>;
  /** Takes an item out of a mission, with an undo. */
  removeFromMission: (m: api.Mission, key: string) => Promise<boolean>;
  /** The items in `close` close, then the mission is done. False when one item did not close. */
  finishMission: (m: api.Mission, close: string[]) => Promise<boolean>;
  addTask: (fields: api.TaskChanges & { title: string }) => Promise<boolean>;
  /** Puts the steps that you picked on the item's task, or on a new task under the item, with an undo. */
  addSteps: (key: string, steps: string[]) => Promise<boolean>;
  /** The mode of the home box: a message to AIfred, or a new task with no AI call. */
  homeMode: "ask" | "task";
  setHomeMode: (m: "ask" | "task") => void;
  editTask: (id: string, changes: api.TaskChanges) => Promise<boolean>;
  tickStep: (id: string, index: number) => void;
  deleteTask: (i: Item) => void;
  tomorrow: (i: Item) => void;
  setMinutes: (key: string, minutes: number) => void;

  /** Saves a new order of the plan of `day` (default today). `toast` says what changed, with an undo. */
  savePlan: (keys: string[], toast?: string, day?: string) => void;
  /** Adds an item to the plan of `day` (default today), at the place `index`, with the fixed time `at`. */
  addToPlan: (key: string, opts?: { index?: number; day?: string; at?: number }) => void;
  removeFromPlan: (key: string, day?: string) => void;
  /** Fixes the start time of a plan item, in minutes after midnight. null: no fixed time. */
  setPlanTime: (key: string, at: number | null, day: string) => void;
  /** Moves a plan item to the plan of another day. */
  moveToDay: (key: string, from: string, to: string, at: number | null) => void;
  startPlan: (mode: "ai" | "self", day?: string) => void;
  /** Removes the plan and the plan chat of `day` (default: the day on the Calendar page). */
  startOver: (day?: string) => void;
  writeBrief: () => void;
  toast: (text: string, undo?: () => void, error?: boolean) => void;
  /** The Google connection. null: not read yet. */
  google: api.GoogleStatus | null;
  /** How each review card shows (Review.tsx). "auto": at its time. "open": you
   * opened it with its button. "hidden": you closed it. It stays when you go to
   * another page, and goes back to "auto" on a new day. */
  reviewCards: { day: CardMode; week: CardMode };
  setReviewCard: (card: "day" | "week", mode: CardMode) => void;
  /** Loads the tasks, the plan, the goals, and the "waiting for" list again, after a change that touches them all. */
  reload: () => void;
  /** Reads the saved ClickUp list again, for example after the question
   * chat marked a ClickUp task done. */
  reloadClickup: () => void;
  /** The question talk on the home page that stays on the screen after
   * AIfred's answer, until you go on: the key and the title of its item.
   * null: the card shows the next question. */
  talk: Talk | null;
  setTalk: (t: Talk | null) => void;
  /** The questions that you skipped on the home page. They go at once,
   * before the brief loads again. */
  skipped: string[];
  setSkipped: (f: (s: string[]) => string[]) => void;
  /** Notes that you answered a question of the brief on the screen. After
   * the last question, AIfred writes the brief and missions again. */
  noteAnswer: () => void;
  goPlan: () => void;
  /** Opens the Tasks page. */
  goTasks: () => void;
};

/** The items that today's brief asks about now, less the ones that you
 * skipped on this screen. An item that is not in the lists is asked about
 * only when its list failed to load: the brief gives its title, and the
 * missions wait for the answer. Else it is closed, or its list still loads.
 * Empty when the brief is not today's. The question card and the step that
 * writes a held brief again use this one rule, so they agree. */
export function openQuestions(
  brief: api.Brief | null,
  byKey: Map<string, Item>,
  today: string,
  skipped: string[],
  githubState: ListState,
  clickupState: ListState,
): string[] {
  const failed = (k: string) => {
    const state = k.startsWith("github:") ? githubState : k.startsWith("clickup:") ? clickupState : "ok";
    return !["loading", "ok", "off"].includes(state);
  };
  if (brief?.day !== today) return [];
  return (brief.ask ?? []).filter((k) => !skipped.includes(k) && (byKey.has(k) ? !byKey.get(k)!.done : failed(k)));
}

export const Ctx = createContext<AppCtx>(null!);
export const useApp = () => useContext(Ctx);
