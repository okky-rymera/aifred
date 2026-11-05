// The data and the actions that all screens share. App.tsx makes them.
import { createContext, useContext } from "react";
import type * as api from "./api";
import type { ChatMsg } from "./Chat";
import type { Item, Schedule } from "./items";

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
  /** "idle", "writing", or the error of the last try. */
  briefState: string;
  githubState: ListState;
  clickupState: ListState;
  githubWarning?: string;
  clickupWarning?: string;
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
  goPlan: () => void;
};

export const Ctx = createContext<AppCtx>(null!);
export const useApp = () => useContext(Ctx);
