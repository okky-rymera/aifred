// The data and the actions that all screens share. App.tsx makes them.
import { createContext, useContext } from "react";
import type * as api from "./api";
import type { ChatMsg } from "./Chat";
import type { Item, Schedule } from "./items";

export type CardMode = "auto" | "open" | "hidden";

export type ListState = "loading" | "ok" | "off" | string;

/** A chat: its messages, and the work in progress. */
export type ChatState = { msgs: ChatMsg[]; busy: boolean; progress: string[] };

export type AppCtx = {
  today: string;
  items: Item[];
  byKey: Map<string, Item>;
  settings: api.Settings;
  plan: api.DayPlan | null;
  /** The plan items that are still open, in order. */
  planItems: Item[];
  sched: Schedule;
  brief: api.Brief | null;
  /** null until the goals are loaded. */
  goals: api.Goals | null;
  /** The text of a goal by its id. */
  goalText: (id: number | undefined) => string | undefined;
  /** Runs one goal change and shows the new goals. False when it failed. */
  changeGoals: (change: () => Promise<api.Goals>) => Promise<boolean>;
  /** "idle", "writing", or the error of the last try. */
  briefState: string;
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
  trackEmail: (m: api.Mail, followUp?: string) => Promise<boolean>;
  hideSent: (m: api.Mail) => Promise<boolean>;
  addWaiting: (item: api.NewWaiting) => Promise<boolean>;
  changeWaiting: (id: number, changes: api.WaitingChanges) => Promise<boolean>;
  removeWaiting: (w: api.Waiting) => Promise<boolean>;
  tasksError: string;

  sel: string | null;
  open: (key: string) => void;
  close: () => void;

  /** The main conversation. */
  main: ChatState;
  mainOpen: boolean;
  openMain: () => void;
  sendMain: (text: string, files: api.FileRef[]) => Promise<boolean>;
  planChat: ChatState;
  sendPlan: (text: string, files?: api.FileRef[]) => Promise<boolean>;
  itemChat: (key: string) => ChatState;
  askItem: (key: string, text: string, files: api.FileRef[]) => Promise<boolean>;

  /** The model of each chat box. null: the default model. */
  models: { main: string | null; plan: string | null; item: string | null };
  setModel: (box: "main" | "plan" | "item", m: string | null) => void;

  toggleDone: (i: Item) => void;
  /** The item that you work on now, or null. */
  work: api.Running | null;
  /** Starts work on an item. The work on another item stops. */
  startWork: (key: string) => void;
  stopWork: () => void;
  /** The "Done" button of the work timer, for the item `key`. */
  finishItem: (key: string) => void;
  /** You say that the brief is wrong about an item. */
  correctBrief: (i: Item, kind: api.CorrectionKind) => void;
  addTask: (fields: api.TaskChanges & { title: string }) => Promise<boolean>;
  /** The mode of the home box: a message to AIfred, or a new task with no AI call. */
  homeMode: "ask" | "task";
  setHomeMode: (m: "ask" | "task") => void;
  editTask: (id: string, changes: api.TaskChanges) => Promise<boolean>;
  tickStep: (id: string, index: number) => void;
  deleteTask: (i: Item) => void;
  tomorrow: (i: Item) => void;
  setMinutes: (key: string, minutes: number) => void;

  /** Saves a new order of the plan. `toast` says what changed, with an undo. */
  savePlan: (keys: string[], toast?: string) => void;
  addToPlan: (key: string, at?: number) => void;
  removeFromPlan: (key: string) => void;
  startPlan: (mode: "ai" | "self") => void;
  startOver: () => void;
  writeBrief: () => void;
  toast: (text: string, undo?: () => void, error?: boolean) => void;
  /** How each review card shows (Review.tsx). "auto": at its time. "open": you
   * opened it with its button. "hidden": you closed it. It stays when you go to
   * another page, and goes back to "auto" on a new day. */
  reviewCards: { day: CardMode; week: CardMode };
  setReviewCard: (card: "day" | "week", mode: CardMode) => void;
  /** Loads the tasks, the plan, the goals, and the "waiting for" list again, after a change that touches them all. */
  reload: () => void;
  goPlan: () => void;
};

export const Ctx = createContext<AppCtx>(null!);
export const useApp = () => useContext(Ctx);
