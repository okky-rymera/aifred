// The text of each Google app that AIfred changes (Gmail drafts, Docs
// suggestions, Sheets comments), in one place. A new app adds one entry here.
import type { Action } from "./api";

export const GOOGLE_APPS: Record<
  Action["app"],
  {
    /** The name of the feature in Settings, as google.rs names it. */
    feature: string;
    /** The app, in "Open in …". */
    name: string;
    /** What Undo does. */
    undo: string;
    /** What an Undo did, when it has no note. */
    undone: string;
    /** What turning the feature off did. */
    off: string;
  }
> = {
  gmail: {
    feature: "Gmail drafts",
    name: "Gmail",
    undo: "Deletes the draft in Gmail. AIfred does not delete a draft that you changed there.",
    undone: "Deleted the draft in Gmail.",
    off: `Gmail drafts is off. AIfred makes no new drafts. Your drafts stay in Gmail. "AIfred's actions" below lets you undo them until you connect Google again.`,
  },
  docs: {
    feature: "Docs suggestions",
    name: "the Doc",
    undo: "Deletes the suggestions that nobody accepted or rejected yet.",
    undone: "Deleted the suggestions in the Doc.",
    off: `Docs suggestions is off. AIfred reads no Docs and makes no new suggestions. Its suggestions stay in the Docs. "AIfred's actions" below lets you undo them until you connect Google again.`,
  },
  sheets: {
    feature: "Sheets comments",
    name: "the Sheet",
    undo: "Deletes the comments that nobody resolved or answered yet.",
    undone: "Deleted the comments in the Sheet.",
    off: `Sheets comments is off. AIfred reads no Sheets and makes no new comments. Its comments stay in the Sheets. "AIfred's actions" below lets you undo them until you connect Google again.`,
  },
};

/** What turning the feature `name` off did. Gmail drafts, Docs
 * suggestions, and Sheets comments remove nothing: they stay in
 * Google, and the action log stays. Undo works until the next login,
 * because a new login does not ask for that access. Other features remove
 * their saved data. */
export const offText = (name: string) =>
  Object.values(GOOGLE_APPS).find((a) => a.feature === name)?.off ?? `${name} is off. AIfred removed the saved data of ${name}.`;
