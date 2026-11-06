// One piece of work in many places (links.rs): a ClickUp task, its GitHub
// issue, and an email about it show as one row in the home list.
// - The "Same work?" card on the home page shows AIfred's suggestions. The
//   user links each pair, or says that they are not the same work.
// - The "Same work" part of the item panel shows the group of the item. The
//   user can link another item, or take an item out of the group.
import { useState } from "react";
import type * as api from "./api";
import { useApp } from "./ctx";
import { SRC_NAME, groupOf, rowKey, type Item } from "./items";

/** How many suggestions the home card shows. */
const CARD_SUGGESTIONS = 3;
/** How many matches the search in the item panel shows. */
const SEARCH_RESULTS = 5;

/** The source and the place of an item, for example "GitHub · shop #7". */
export const srcText = (i: Item) => `${SRC_NAME[i.src]}${i.ref ? ` · ${i.ref}` : ""}`;

function Pair({ s }: { s: api.LinkSuggestion }) {
  const { links, byKey, linkItems, rejectLink, open } = useApp();
  const a = byKey.get(s.main)!;
  const b = byKey.get(s.key)!;
  // The link puts both groups under the main item of the group of `s.main` (links.rs).
  const under = byKey.get(rowKey(groupOf(links.links, s.main), byKey)) ?? a;
  const name = (i: Item) => (
    <button className="linkbtn ttl" onClick={() => open(i.key)} title={srcText(i)}>
      {i.title} <span className={`src ${i.src}`}>{SRC_NAME[i.src]}</span>
    </button>
  );
  return (
    <li className="w-row">
      <span className="w-main">
        <span className="w-line">
          {name(a)} and {name(b)}
        </span>
        <span className="w-why">{s.why}</span>
      </span>
      <button className="btn" aria-label={`Link: ${a.title} and ${b.title}`} title={`Show them as one row, under "${under.title}"`} onClick={() => linkItems(s.main, s.key)}>
        Link
      </button>
      <button className="linkbtn" aria-label={`Not the same: ${a.title} and ${b.title}`} title="AIfred does not suggest these two again" onClick={() => rejectLink(s)}>
        Not the same
      </button>
    </li>
  );
}

/** AIfred's suggestions: two items that look like one piece of work. */
export function LinksCard() {
  const { links, byKey } = useApp();
  const list = links.suggestions.filter((s) => byKey.has(s.main) && byKey.has(s.key));
  if (!list.length) return null;
  return (
    <section className="waiting" aria-label="Same work?">
      <div className="eyebrow">
        <span>🔗 Same work? · {list.length}</span>
      </div>
      <ul className="w-list" aria-label="Suggested links">
        {list.slice(0, CARD_SUGGESTIONS).map((s) => (
          <Pair key={`${s.main}|${s.key}`} s={s} />
        ))}
      </ul>
    </section>
  );
}

/** Finds an open item by its title, to link it. */
function Search({ item, group, onDone }: { item: Item; group: string[]; onDone: () => void }) {
  const { items, linkItems } = useApp();
  const [q, setQ] = useState("");
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const found = words.length
    ? items.filter((i) => !i.done && !group.includes(i.key) && words.every((w) => `${i.title} ${i.ref}`.toLowerCase().includes(w))).slice(0, SEARCH_RESULTS)
    : [];
  return (
    <div className="same-search">
      <input aria-label="Find an item to link" placeholder="Find an item by its title" value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
      {found.map((i) => (
        <div key={i.key} className="w-row">
          <span className="w-main">
            <span className="w-line">{i.title}</span>
            <span className="w-why">{srcText(i)}</span>
          </span>
          <button
            className="btn"
            aria-label={`Link: ${i.title}`}
            onClick={async () => {
              if (await linkItems(item.key, i.key)) onDone();
            }}
          >
            Link
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

/** The "Same work" part of the item panel. */
export function SameWork({ item }: { item: Item }) {
  const { links, byKey, unlinkItem, open } = useApp();
  const [adding, setAdding] = useState(false);
  const group = groupOf(links.links, item.key);
  const others = group.filter((k) => k !== item.key);
  // The same item as the row in the home list (joinLinked).
  const row = rowKey(group, byKey);
  const sugg = links.suggestions.filter((s) => (s.main === item.key || s.key === item.key) && byKey.has(s.main) && byKey.has(s.key));
  if (item.done && !others.length) return null;
  return (
    <>
      <div className="d-sec">
        Same work {others.length > 0 && <span className="c">{group.length}</span>}
      </div>
      <div className="same">
        {others.length > 0 && (
          <ul className="w-list" aria-label="Same work">
            {group.map((k) => {
              const i = byKey.get(k);
              const name = i?.title ?? k;
              return (
                <li key={k} className="w-row">
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
                    </span>
                  </span>
                  <button className="linkbtn" aria-label={`Unlink: ${name}`} title="Show it as its own row again" onClick={() => unlinkItem(k)}>
                    Unlink
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        {sugg.length > 0 && (
          <ul className="w-list" aria-label="Suggested links">
            {sugg.map((s) => (
              <Pair key={`${s.main}|${s.key}`} s={s} />
            ))}
          </ul>
        )}
        {adding ? (
          <Search item={item} group={group} onDone={() => setAdding(false)} />
        ) : (
          !item.done && (
            <button className="linkbtn" style={{ alignSelf: "flex-start" }} onClick={() => setAdding(true)} title="The same work in another place, for example the GitHub pull request of a ClickUp task">
              + Link another item
            </button>
          )
        )}
      </div>
    </>
  );
}
