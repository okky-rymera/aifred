// Subtasks (links.rs): an item under another item, in AIfred only. A
// ClickUp task, its GitHub issue, and the pull request show as one row in
// the home list. ClickUp and GitHub do not change.
// - The "Subtasks" part of the item panel shows the tree of the item. The
//   user can add a subtask, or move an item out from under its parent.
import { useState } from "react";
import { useApp } from "./ctx";
import { SRC_NAME, depthOf, groupOf, parentOf, rowKey, type Item } from "./items";

/** How many matches the search in the item panel shows. */
const SEARCH_RESULTS = 5;

/** The source and the place of an item, for example "GitHub · shop #7". */
export const srcText = (i: Item) => `${SRC_NAME[i.src]}${i.ref ? ` · ${i.ref}` : ""}`;

/** Finds an open item by its title, to make it a subtask of `item`. */
function Search({ item, onDone }: { item: Item; onDone: () => void }) {
  const { items, links, linkItems } = useApp();
  // Not the item, not its parents (an item cannot go under its own subtask),
  // and not its subtasks of now.
  const skip = new Set([item.key, ...links.links.filter((l) => l.main === item.key).map((l) => l.key)]);
  for (let p = parentOf(links.links, item.key); p && !skip.has(p); p = parentOf(links.links, p)) skip.add(p);
  const [q, setQ] = useState("");
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const found = words.length
    ? items.filter((i) => !i.done && !skip.has(i.key) && words.every((w) => `${i.title} ${i.ref}`.toLowerCase().includes(w))).slice(0, SEARCH_RESULTS)
    : [];
  return (
    <div className="same-search">
      <input aria-label="Find an item to add as a subtask" placeholder="Find an item by its title" value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
      {found.map((i) => (
        <div key={i.key} className="w-row">
          <span className="w-main">
            <span className="w-line">{i.title}</span>
            <span className="w-why">{srcText(i)}</span>
          </span>
          <button
            className="btn"
            aria-label={`Add as subtask: ${i.title}`}
            title={`Put it under "${item.title}", with its own subtasks`}
            onClick={async () => {
              if (await linkItems(item.key, i.key)) onDone();
            }}
          >
            Add
          </button>
        </div>
      ))}
      {words.length > 0 && !found.length && <div className="w-why">No open item matches.</div>}
      <button className="linkbtn" onClick={onDone}>
        Cancel
      </button>
    </div>
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
                <li key={k} className="w-row" style={{ paddingLeft: depthOf(links.links, k) * 16 }}>
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
