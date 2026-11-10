// Subtasks (links.rs): an item under another item, in AIfred only. A
// ClickUp task, its GitHub issue, and the pull request show as one row in
// the home list. ClickUp and GitHub do not change.
// - The "Subtasks" part of the item panel shows the tree of the item. The
//   user can add a subtask, or move an item out from under its parent.
// Blockers (blockers.rs): an item that waits until other items are finished.
// - The "Blocked by" part of the item panel shows what the item waits for,
//   and what waits for it. A blocked row shows "Blocked by" with the open
//   blockers. The user can still finish a blocked item.
import { useState } from "react";
import { useApp } from "./ctx";
import { SRC_NAME, groupOf, parentOf, rowKey, treeDepths, type Item } from "./items";

/** How many matches the search in the item panel shows. */
const SEARCH_RESULTS = 5;

/** The source and the place of an item, for example "GitHub · shop #7". */
export const srcText = (i: Item) => `${SRC_NAME[i.src]}${i.ref ? ` · ${i.ref}` : ""}`;

/** Finds an open item by its title, and runs `pick` on it. `skip` are the
 * items that the search must not show. */
export function FindItem(props: { skip: Set<string>; label: string; act: string; actTitle: (i: Item) => string; pick: (i: Item) => Promise<boolean>; onDone: () => void }) {
  const { items } = useApp();
  const [q, setQ] = useState("");
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const found = words.length
    ? items.filter((i) => !i.done && !props.skip.has(i.key) && words.every((w) => `${i.title} ${i.ref}`.toLowerCase().includes(w))).slice(0, SEARCH_RESULTS)
    : [];
  return (
    <div className="same-search">
      <input aria-label={props.label} placeholder="Find an item by its title" value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
      {found.map((i) => (
        <div key={i.key} className="w-row">
          <span className="w-main">
            <span className="w-line">{i.title}</span>
            <span className="w-why">{srcText(i)}</span>
          </span>
          <button
            className="btn"
            aria-label={`${props.act}: ${i.title}`}
            title={props.actTitle(i)}
            onClick={async () => {
              if (await props.pick(i)) props.onDone();
            }}
          >
            Add
          </button>
        </div>
      ))}
      {words.length > 0 && !found.length && <div className="w-why">No open item matches.</div>}
      <button className="linkbtn" onClick={props.onDone}>
        Cancel
      </button>
    </div>
  );
}

/** Finds an open item by its title, to make it a subtask of `item`. */
function Search({ item, onDone }: { item: Item; onDone: () => void }) {
  const { links, linkItems } = useApp();
  // Not the item, not its parents (an item cannot go under its own subtask),
  // and not its subtasks of now.
  const skip = new Set([item.key, ...links.links.filter((l) => l.main === item.key).map((l) => l.key)]);
  for (let p = parentOf(links.links, item.key); p && !skip.has(p); p = parentOf(links.links, p)) skip.add(p);
  return (
    <FindItem
      skip={skip}
      label="Find an item to add as a subtask"
      act="Add as subtask"
      actTitle={() => `Put it under "${item.title}", with its own subtasks`}
      pick={(i) => linkItems(item.key, i.key)}
      onDone={onDone}
    />
  );
}

/** The "Subtasks" part of the item panel: the tree of the item, the top
 * item first, each subtask under its parent. */
export function SameWork({ item }: { item: Item }) {
  const { links, byKey, unlinkItem, open } = useApp();
  const [adding, setAdding] = useState(false);
  const group = groupOf(links.links, item.key);
  const others = group.filter((k) => k !== item.key);
  // The same item as the row in the home list (joinLinked).
  const row = rowKey(group, byKey);
  const depth = treeDepths(links.links, group);
  if (item.done && !others.length) return null;
  const nameOf = (k: string) => byKey.get(k)?.title ?? k;
  return (
    <>
      <div className="d-sec">
        Subtasks {others.length > 0 && <span className="c">{group.length}</span>}
      </div>
      <div className="same">
        {others.length > 0 && (
          <ul className="w-list" aria-label="Task tree">
            {group.map((k) => {
              const i = byKey.get(k);
              const name = nameOf(k);
              const parent = parentOf(links.links, k);
              const link = links.links.find((l) => l.key === k);
              return (
                <li key={k} className="w-row" style={{ paddingLeft: (depth.get(k) ?? 0) * 16 }}>
                  <span className="w-main">
                    {k === item.key ? (
                      <span className="w-line">This item</span>
                    ) : (
                      <button className="linkbtn ttl w-line" onClick={() => open(k)} aria-label={`Open details: ${name}`}>
                        {name}
                      </button>
                    )}
                    <span className="w-why">
                      {i ? srcText(i) : "Not in your list now"}
                      {k === row ? " · main row" : ""}
                      {i?.done ? " · finished" : ""}
                      {link?.by === "aifred" && link.why ? ` · AIfred: ${link.why}` : ""}
                    </span>
                  </span>
                  {parent && (
                    <button className="linkbtn" aria-label={`Move out: ${name}`} title={`Take it out from under "${nameOf(parent)}", with its own subtasks`} onClick={() => unlinkItem(k)}>
                      Move out
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
        {adding ? (
          <Search item={item} onDone={() => setAdding(false)} />
        ) : (
          !item.done && (
            <button className="linkbtn" style={{ alignSelf: "flex-start" }} onClick={() => setAdding(true)} title="Put another item under this one, for example the GitHub pull request of a ClickUp task">
              + Add a subtask
            </button>
          )
        )}
      </div>
    </>
  );
}

/** The open items that `key` waits for, in the order of the links. */
export function useOpenBlockers(key: string): Item[] {
  const { blockers, byKey } = useApp();
  return blockers.blockers.flatMap((b) => {
    const i = b.key === key ? byKey.get(b.blocker) : undefined;
    return i && !i.done ? [i] : [];
  });
}

/** "Blocked by X" on a row: the open items that this item waits for. */
export function BlockedMark({ item }: { item: Item }) {
  const { open } = useApp();
  const list = useOpenBlockers(item.key);
  if (item.done || !list.length) return null;
  return (
    <div className="blocked-by">
      <span>Blocked by</span>
      {list.map((b) => (
        <button key={b.key} className="linkbtn" title={srcText(b)} aria-label={`Open blocker: ${b.title}`} onClick={() => open(b.key)}>
          {b.title}
        </button>
      ))}
    </div>
  );
}

/** One line of the "Blocked by" part: an item, where it is from, and who set the link. */
function BlockerLine({ k, link, remove, removeLabel }: { k: string; link: { by: string; why: string }; remove: () => void; removeLabel: string }) {
  const { byKey, open } = useApp();
  const i = byKey.get(k);
  const name = i?.title ?? k;
  return (
    <li className="w-row">
      <span className="w-main">
        <button className="linkbtn ttl w-line" onClick={() => open(k)} aria-label={`Open details: ${name}`}>
          {name}
        </button>
        <span className="w-why">
          {i ? srcText(i) : "Not in your list now"}
          {i?.done ? " · finished" : ""}
          {link.by === "aifred" && link.why ? ` · AIfred: ${link.why}` : ""}
          {link.by === "chat" ? " · Set by AIfred in a chat" : ""}
        </span>
      </span>
      <button className="linkbtn" aria-label={removeLabel} title="Take the link away. AIfred does not add it again." onClick={remove}>
        Remove
      </button>
    </li>
  );
}

/** The "Blocked by" part of the item panel: the items that this item waits
 * for, and the items that wait for it. */
export function BlockedBy({ item }: { item: Item }) {
  const { blockers, addBlocker, removeBlocker, byKey } = useApp();
  const [adding, setAdding] = useState(false);
  // The same count as the row mark: a blocker that is not in the list now
  // does not count (useOpenBlockers, and blockers.rs open_blockers_in).
  const open = useOpenBlockers(item.key).length;
  const waitsFor = blockers.blockers.filter((b) => b.key === item.key);
  const blocks = blockers.blockers.filter((b) => b.blocker === item.key);
  if (item.done && !waitsFor.length && !blocks.length) return null;
  const nameOf = (k: string) => byKey.get(k)?.title ?? k;
  // Not the item, not its blockers of now, and not the items that wait for
  // it, also through other items: two items cannot wait for each other.
  // Like blockers.rs, only a link to an open item counts.
  const isOpen = (k: string) => {
    const i = byKey.get(k);
    return !!i && !i.done;
  };
  const skip = new Set([item.key, ...waitsFor.map((b) => b.blocker)]);
  for (const todo = [item.key]; todo.length; ) {
    const k = todo.pop()!;
    if (!isOpen(k)) continue;
    for (const b of blockers.blockers) {
      if (b.blocker === k && !skip.has(b.key)) {
        skip.add(b.key);
        todo.push(b.key);
      }
    }
  }
  return (
    <>
      <div className="d-sec">Blocked by {open > 0 && <span className="c">{open}</span>}</div>
      <div className="same">
        {waitsFor.length > 0 && (
          <ul className="w-list" aria-label="Blocked by">
            {waitsFor.map((b) => (
              <BlockerLine key={b.blocker} k={b.blocker} link={b} remove={() => removeBlocker(item.key, b.blocker)} removeLabel={`Not blocked by: ${nameOf(b.blocker)}`} />
            ))}
          </ul>
        )}
        {adding ? (
          <FindItem
            skip={skip}
            label="Find an item that must finish first"
            act="Blocked by"
            actTitle={(i) => `"${item.title}" waits until "${i.title}" is finished`}
            pick={(i) => addBlocker(item.key, i.key)}
            onDone={() => setAdding(false)}
          />
        ) : (
          !item.done && (
            <button className="linkbtn" style={{ alignSelf: "flex-start" }} onClick={() => setAdding(true)} title="Pick an item that must be finished before this one">
              + Add a blocker
            </button>
          )
        )}
        {blocks.length > 0 && (
          <>
            <div className="w-why">This item blocks:</div>
            <ul className="w-list" aria-label="Blocks">
              {blocks.map((b) => (
                <BlockerLine key={b.key} k={b.key} link={b} remove={() => removeBlocker(b.key, item.key)} removeLabel={`Does not block: ${nameOf(b.key)}`} />
              ))}
            </ul>
          </>
        )}
      </div>
    </>
  );
}
