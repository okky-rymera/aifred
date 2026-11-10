// AIfred's own status of each item (status.rs): To do, In progress,
// Waiting, Done, Unfinished, Cancelled. It is the same for all sources. A
// ClickUp status name can mean another thing in each list, so it is only a
// detail here. The status is only in AIfred: only Done of a ClickUp task
// changes ClickUp (it is "mark done").
//
// The Settings part follows the "group defaults plus exceptions" pattern of
// status mapping screens (for example Atlassian's status categories, and
// Mosaic's mapping editor): each ClickUp status group has a fixed AIfred
// status, and a table of overrides changes single statuses.
import { useEffect, useMemo, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import type { Item } from "./items";
import { FindItem, srcText } from "./Links";

/** Why the item has its status, in a few words. */
export function statusWhy(i: Item): string {
  const src = i.statusSource ? `"${i.statusSource}"` : "";
  switch (i.statusBy) {
    case "you":
      return "set by you";
    case "source":
      return src ? `closed as ${src}` : "closed there";
    case "rule":
      return `your override for ${src || "this status"}`;
    case "delegate":
      return "given away in the weekly review";
    case "blocker":
      return "waits for another item";
    case "timer":
      return "you started the work";
    default:
      return i.cu && src ? `from ${src}` : "";
  }
}

/** "Continued in X" or "Accepted as it is", for an Unfinished item. */
export function nextText(i: Item): string {
  if (i.status !== "unfinished") return "";
  return i.statusNext
    ? `Continued in "${i.statusNext.title ?? i.statusNext.key}"`
    : "Accepted as it is";
}

/** The status of a row. To do shows nothing, except for ClickUp, where it
 * replaces the ClickUp status name (the name is in the tooltip). */
export function StatusPill({ item: i }: { item: Item }) {
  if (i.mail || (i.status === "todo" && !i.cu)) return null;
  const why = statusWhy(i);
  const title = [
    i.cu ? `ClickUp status: ${i.cu.status}.` : "",
    api.STATUS_HINT[i.status],
    why ? `(${why})` : "",
    nextText(i),
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <span className={`pill status st-${i.status}`} title={title}>
      {i.cu && (
        <i
          style={{ background: i.cu.status_color || "var(--faint)" }}
          aria-hidden
        />
      )}
      {api.STATUS_NAME[i.status]}
    </span>
  );
}

/** The status menu of the item panel. */
export function StatusPicker({ item: i }: { item: Item }) {
  const { setStatus } = useApp();
  const [busy, setBusy] = useState(false);
  if (i.mail) return null;
  const why = statusWhy(i);
  return (
    <span className="ctl">
      <select
        aria-label="Status"
        value={i.status}
        disabled={busy}
        title={api.STATUS_HINT[i.status]}
        onChange={async (e) => {
          setBusy(true);
          try {
            await setStatus(i, e.target.value as api.Status);
          } finally {
            setBusy(false);
          }
        }}
      >
        {api.STATUSES.map((s) => (
          <option key={s} value={s} title={api.STATUS_HINT[s]}>
            {api.STATUS_NAME[s]}
            {s === "done" && i.cu ? " (closes it in ClickUp)" : ""}
          </option>
        ))}
      </select>
      {why && <span className="hintx">{why}</span>}
    </span>
  );
}

/** One linked item: its title opens it. */
function LinkedLine({
  l,
  why,
  action,
}: {
  l: api.Linked;
  why?: string;
  action?: React.ReactNode;
}) {
  const { byKey, open } = useApp();
  const i = byKey.get(l.key);
  const name = i?.title ?? l.title ?? l.key;
  return (
    <li className="w-row">
      <span className="w-main">
        <button
          className="linkbtn ttl w-line"
          onClick={() => open(l.key)}
          aria-label={`Open details: ${name}`}
        >
          {name}
        </button>
        <span className="w-why">
          {i ? srcText(i) : "Not in your list now"}
          {why ? ` · ${why}` : ""}
        </span>
      </span>
      {action}
    </li>
  );
}

/** The "Continued in" part of the item panel. An Unfinished item links to
 * the item that continues it (for example the copy of a rock in the next
 * quarter). With no link, you accept it as it is. The item that continues
 * it shows "Continues". */
export function ContinuedIn({ item: i }: { item: Item }) {
  const { setStatusNext } = useApp();
  const [adding, setAdding] = useState(false);
  const continues = i.statusContinues ?? [];
  if (i.status !== "unfinished" && !continues.length) return null;
  return (
    <>
      {i.status === "unfinished" && (
        <>
          <div className="d-sec">Continued in</div>
          <div className="same">
            {i.statusNext ? (
              <ul className="w-list" aria-label="Continued in">
                <LinkedLine
                  l={i.statusNext}
                  action={
                    <button
                      className="linkbtn"
                      aria-label="Remove the link: accept it as it is"
                      title="It was not continued: accept it as it is"
                      onClick={() => setStatusNext(i, null)}
                    >
                      Remove
                    </button>
                  }
                />
              </ul>
            ) : (
              <div className="w-why">
                Accepted as it is. If another task continues this work, link it
                here.
              </div>
            )}
            {adding ? (
              <FindItem
                skip={new Set([i.key])}
                label="Find the task that continues it"
                act="Continued in"
                actTitle={(n) => `"${n.title}" continues "${i.title}"`}
                pick={(n) => setStatusNext(i, n)}
                onDone={() => setAdding(false)}
              />
            ) : (
              <button
                className="linkbtn"
                style={{ alignSelf: "flex-start" }}
                onClick={() => setAdding(true)}
              >
                {i.statusNext
                  ? "Change the task"
                  : "+ Link the task that continues it"}
              </button>
            )}
          </div>
        </>
      )}
      {continues.length > 0 && (
        <>
          <div className="d-sec">Continues</div>
          <div className="same">
            <ul className="w-list" aria-label="Continues">
              {continues.map((l) => (
                <LinkedLine key={l.key} l={l} why="Unfinished" />
              ))}
            </ul>
          </div>
        </>
      )}
    </>
  );
}

// ---------- Settings: overrides for ClickUp statuses ----------

const INPUT =
  "rounded-lg border border-stone-300 bg-white px-2 py-1 text-sm text-stone-900 dark:border-stone-700 dark:bg-stone-900 dark:text-stone-100";
const BTN =
  "rounded-lg border border-stone-300 px-3 py-1 text-sm hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800";
const TH = "px-2 py-1.5 text-left text-xs font-medium text-stone-500";
const TD = "px-2 py-1.5 align-middle";

/** The fixed defaults: each ClickUp group and its AIfred status (status.rs default_for). */
const DEFAULTS: { group: string; status: api.Status }[] = [
  { group: "Not started", status: api.groupDefault("open") },
  { group: "Active", status: api.groupDefault("custom") },
  { group: "Done", status: api.groupDefault("done") },
  { group: "Closed", status: api.groupDefault("closed") },
];

/** A ClickUp status with its color dot and its group. */
function CuStatusName({
  name,
  kind,
  color,
}: {
  name: string;
  kind: string;
  color?: string;
}) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <i
        className="inline-block size-2 rounded-full"
        style={{ background: color || "var(--faint)" }}
        aria-hidden
      />
      <span>{name}</span>
      {kind && (
        <span className="text-xs text-stone-500">
          ({api.CU_GROUP[kind] ?? kind})
        </span>
      )}
    </span>
  );
}

/** The status names to offer for "Any list": the space's own ones and the
 * ones of each list in it (a list can have its own statuses). */
function anyListStatuses(s: api.SpaceStatuses): api.CuStatus[] {
  const out: api.CuStatus[] = [];
  for (const st of [...s.statuses, ...s.lists.flatMap((l) => l.statuses)]) {
    if (!out.some((o) => o.name.toLowerCase() === st.name.toLowerCase()))
      out.push(st);
  }
  return out;
}

/** The form row that adds one override: Space, List, ClickUp status, AIfred status. */
function AddRule({
  onAdded,
  onCancel,
}: {
  onAdded: (r: api.StatusRule) => Promise<void>;
  onCancel: () => void;
}) {
  const [spaces, setSpaces] = useState<api.CuSpace[] | null>(null);
  const [spaceId, setSpaceId] = useState("");
  const [detail, setDetail] = useState<api.SpaceStatuses | null>(null);
  const [listId, setListId] = useState("");
  const [name, setName] = useState("");
  const [status, setStatus] = useState<api.Status | "">("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .clickupSpaces()
      .then(setSpaces)
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(() => {
    setDetail(null);
    setListId("");
    setName("");
    if (!spaceId) return;
    let live = true;
    api
      .clickupSpaceStatuses(spaceId)
      .then((d) => live && setDetail(d))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [spaceId]);

  const list = detail?.lists.find((l) => l.id === listId);
  const choices = detail
    ? list
      ? list.statuses
      : anyListStatuses(detail)
    : [];
  const picked = choices.find((c) => c.name === name);
  const space = spaces?.find((s) => s.id === spaceId);
  const multiWorkspace = new Set(spaces?.map((s) => s.workspace)).size > 1;

  async function add() {
    if (!space || !picked || !status) return;
    setBusy(true);
    setError("");
    try {
      await onAdded({
        name: picked.name,
        scope: list ? "list" : "space",
        scope_id: list ? list.id : space.id,
        scope_name: list ? list.name : space.name,
        space_id: space.id,
        space_name: space.name,
        kind: picked.kind,
        status,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      role="group"
      aria-label="New override"
      className="mt-2 space-y-3 rounded-lg border border-stone-200 p-3 dark:border-stone-800"
    >
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label className="flex flex-col gap-1 text-xs text-stone-500">
          Space
          <select
            aria-label="Space"
            value={spaceId}
            onChange={(e) => setSpaceId(e.target.value)}
            className={INPUT}
            disabled={!spaces}
          >
            <option value="">{spaces ? "Choose a space" : "Loading…"}</option>
            {spaces?.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
                {multiWorkspace ? ` (${s.workspace})` : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs text-stone-500">
          List
          <select
            aria-label="List"
            value={listId}
            onChange={(e) => {
              setListId(e.target.value);
              setName("");
            }}
            className={INPUT}
            disabled={!detail}
          >
            <option value="">
              {spaceId && !detail ? "Loading…" : "Any list in the space"}
            </option>
            {detail?.lists.map((l) => (
              <option key={l.id} value={l.id}>
                {l.folder ? `${l.folder} / ` : ""}
                {l.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs text-stone-500">
          ClickUp status
          <select
            aria-label="ClickUp status"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              const c = choices.find((x) => x.name === e.target.value);
              if (c && !status) setStatus(api.groupDefault(c.kind));
            }}
            className={INPUT}
            disabled={!detail}
          >
            <option value="">Choose a status</option>
            {choices.map((c) => (
              <option key={c.name} value={c.name}>
                {c.name} ({api.CU_GROUP[c.kind] ?? c.kind})
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs text-stone-500">
          AIfred status
          <select
            aria-label="AIfred status"
            value={status}
            onChange={(e) => setStatus(e.target.value as api.Status)}
            className={INPUT}
          >
            <option value="">Choose</option>
            {api.STATUSES.map((s) => (
              <option key={s} value={s} title={api.STATUS_HINT[s]}>
                {api.STATUS_NAME[s]}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div>
        <div className="flex items-center gap-2">
          <button
            className={BTN}
            onClick={add}
            disabled={busy || !picked || !status}
          >
            Add
          </button>
          <button className="text-sm underline" onClick={onCancel}>
            Cancel
          </button>
        </div>
        {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
      </div>
    </div>
  );
}

/** Where a rule counts, for the List column. */
function ruleList(r: api.StatusRule): string {
  switch (r.scope) {
    case "list":
      return r.scope_name || r.scope_id;
    case "space":
      return "Any list in the space";
    case "folder":
      return `Folder: ${r.scope_name || r.scope_id}`;
    default:
      return "All lists";
  }
}

/** What each ClickUp status means in AIfred: the group defaults, and your
 * overrides. The overrides change only AIfred, not ClickUp. */
export function ClickupStatusSection({
  onChanged,
}: {
  onChanged?: () => void;
}) {
  const [rules, setRules] = useState<api.StatusRule[] | null>(null);
  const [use, setUse] = useState<Record<string, number>>({});
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState<{ text: string; undo?: () => void } | null>(
    null,
  );

  const loadUse = () =>
    api
      .statusRuleUse()
      .then(setUse)
      .catch(() => {});
  useEffect(() => {
    api
      .statusRules()
      .then(setRules)
      .catch((e) => setError(String(e)));
    void loadUse();
  }, []);

  /** Shows the new rules, and tells the app that the statuses changed. */
  const changed = (
    all: api.StatusRule[],
    text: string,
    undo?: () => Promise<void>,
  ) => {
    setRules(all);
    setError("");
    void loadUse();
    onChanged?.();
    setNote({
      text,
      undo: undo && (() => void undo().catch((e) => setError(String(e)))),
    });
  };
  const undoTo =
    (r: api.StatusRule, before: api.StatusRule | null) => async () => {
      const all = before
        ? (await api.setStatusRule(before))[0]
        : await api.deleteStatusRule(r);
      changed(all, "Undone.");
    };
  const describe = (r: api.StatusRule) =>
    `"${r.name}" in ${r.scope === "space" ? `any list of ${r.space_name}` : r.scope_name}`;

  async function save(r: api.StatusRule) {
    const [all, before] = await api.setStatusRule(r);
    changed(
      all,
      `${describe(r)} is ${api.STATUS_NAME[r.status]} now.`,
      undoTo(r, before),
    );
  }

  async function remove(r: api.StatusRule) {
    try {
      const all = await api.deleteStatusRule(r);
      changed(all, `The override for ${describe(r)} is removed.`, async () =>
        changed((await api.setStatusRule(r))[0], "Undone."),
      );
    } catch (e) {
      setError(String(e));
    }
  }

  const sorted = useMemo(() => rules ?? [], [rules]);

  return (
    <section className="space-y-4">
      <div className="space-y-2">
        <h2 className="text-sm font-semibold">ClickUp statuses</h2>
        <p className="text-sm text-stone-500">
          AIfred has its own status for each item. For a ClickUp task, the
          ClickUp status group decides it. Add an override when one ClickUp
          status means another thing for you. Overrides change only AIfred, not
          ClickUp.
        </p>
      </div>

      <div className="space-y-1">
        <h3 className="text-sm font-medium">Defaults</h3>
        <table className="text-sm" aria-label="Defaults">
          <thead>
            <tr>
              <th className={TH}>ClickUp group</th>
              <th className={TH}>AIfred status</th>
            </tr>
          </thead>
          <tbody>
            {DEFAULTS.map((d) => (
              <tr
                key={d.group}
                className="border-t border-stone-200 dark:border-stone-800"
              >
                <td className={TD}>{d.group}</td>
                <td className={TD}>{api.STATUS_NAME[d.status]}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="space-y-1">
        <h3 className="text-sm font-medium">Overrides</h3>
        <p className="text-sm text-stone-500">
          An override for &quot;Any list in the space&quot; also covers the new
          lists in that space, for example the list of the next quarter. An
          override for one list wins over one for its space.
        </p>
        {note && (
          <p role="status" className="text-sm text-stone-500">
            {note.text}{" "}
            {note.undo && (
              <button
                className="underline"
                onClick={() => {
                  const u = note.undo!;
                  setNote(null);
                  u();
                }}
              >
                Undo
              </button>
            )}
          </p>
        )}
        {error && <p className="text-sm text-red-600">{error}</p>}
        <div className="overflow-x-auto">
          <table className="w-full text-sm" aria-label="Overrides">
            <thead>
              <tr>
                <th className={TH}>Space</th>
                <th className={TH}>List</th>
                <th className={TH}>ClickUp status</th>
                <th className={TH}>AIfred status</th>
                <th
                  className={TH}
                  title="How many of your ClickUp tasks it changes now"
                >
                  Tasks
                </th>
                <th className={TH}>
                  <span className="sr-only">Remove</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((r) => (
                <tr
                  key={r.id ?? `${r.name}:${r.scope}:${r.scope_id}`}
                  className="border-t border-stone-200 dark:border-stone-800"
                >
                  <td className={TD}>{r.space_name || "—"}</td>
                  <td className={TD}>{ruleList(r)}</td>
                  <td className={TD}>
                    <CuStatusName name={r.name} kind={r.kind} />
                  </td>
                  <td className={TD}>
                    <select
                      aria-label={`AIfred status for ${r.name} in ${ruleList(r)}`}
                      value={r.status}
                      onChange={(e) =>
                        save({
                          ...r,
                          status: e.target.value as api.Status,
                        }).catch((x) => setError(String(x)))
                      }
                      className={INPUT}
                    >
                      {api.STATUSES.map((s) => (
                        <option key={s} value={s} title={api.STATUS_HINT[s]}>
                          {api.STATUS_NAME[s]}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td className={`${TD} text-stone-500`}>
                    {r.id !== undefined ? (use[String(r.id)] ?? 0) : ""}
                  </td>
                  <td className={TD}>
                    <button
                      className="text-sm underline"
                      aria-label={`Remove the override for ${r.name} in ${ruleList(r)}`}
                      onClick={() => remove(r)}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              ))}
              {rules?.length === 0 && !adding && (
                <tr className="border-t border-stone-200 dark:border-stone-800">
                  <td className={`${TD} text-stone-500`} colSpan={6}>
                    No overrides. Each ClickUp status gets the default of its
                    group.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {adding && (
          <AddRule
            onAdded={async (r) => {
              await save(r);
              setAdding(false);
            }}
            onCancel={() => setAdding(false)}
          />
        )}
        {!adding && (
          <button
            className={BTN}
            onClick={() => setAdding(true)}
            disabled={rules === null}
          >
            + Add override
          </button>
        )}
      </div>
    </section>
  );
}
