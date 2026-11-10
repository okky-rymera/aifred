# Research: graph memory for AIfred

Date: 2026-11-10.

Question: Can a knowledge graph help AIfred's memory? Must it replace the meaning search (vectors), or must it be added on top?

## Answer

- Add a graph on top of the current search. Do not replace the vectors.
- All maintained graph memory projects keep meaning search, and most of them also keep word search. Only Graphify has no vectors, and it is a tool for code, not for chat memory.
- Do not use a full graph memory project (Graphiti, LightRAG, GraphRAG). Each one makes several AI calls for each message. That uses the user's Claude quota. The published gains are small or mixed.
- Add small graph tables to `aifred.db` instead. Fill them from data that AIfred has already, and from AI calls that AIfred makes already.

## What AIfred has now

- Word search (FTS5) and meaning search (sqlite-vec, all-MiniLM-L6-v2) for facts, chat messages, chat summaries, tasks, and GitHub and ClickUp items (`crates/aifred-core/src/db.rs:95-198`).
- The two result lists are merged with reciprocal rank fusion (RRF): each row gets 1 / (60 + rank) from each list (`db.rs:1195`).
- Some graph parts exist already:
  - `item_link`: the subtask tree of items (`db.rs:579`, SCHEMA_22).
  - `mission` and `mission_item`: groups of items that are one unit of work (`db.rs:721`).
  - `item_context`: short notes about one item (`db.rs:817`).
  - `waiting.wait_for`: the people that an email waits for (`db.rs:486`).
- Two parts are missing:
  - A record for each person, project, or client, with links to the facts, chats, emails, and tasks about it.
  - A time model for facts. A new fact ("I moved to Sydney") does not mark the old fact ("I live in Melbourne") as replaced. The `memory` table has no "replaced by" or "valid until" field (`db.rs:22`).

## Projects

| Project | Graph content and build | Search | Storage, license, state |
|---|---|---|---|
| [Graphify](https://github.com/Graphify-Labs/graphify) | Code: tree-sitter, no AI. Docs: one AI pass. | Graph walk only. "No embeddings, no vector store." | `graph.json`. Python. Made for code and docs. |
| [Graphiti / Zep](https://github.com/getzep/graphiti) | People, things, and facts as edges, with valid and invalid times. About 6 AI stages for each message ([paper](https://arxiv.org/html/2501.13956v1)). | Vectors + BM25 + graph walk, then rerank. | Neo4j, FalkorDB, Neptune. Python, Apache-2.0. |
| [mem0 graph](https://docs.mem0.ai/open-source/features/graph-memory) | AI extracts entities and relations. | Graph gives a "ranking boost" "combined with semantic (vector) and keyword (BM25) scores". | The open-source graph store was removed ([migration](https://docs.mem0.ai/migration/oss-v2-to-v3)). Now a paid platform feature. |
| [Cognee](https://github.com/topoteretes/cognee) | Entities, relations, chunks. AI or GLiNER. | Graph, vector, or code, by question. | Postgres option. Apache-2.0. Has a Rust port. |
| [LightRAG](https://github.com/HKUDS/LightRAG) | One AI call for each chunk, plus merge calls. | Graph + vectors ("mix" mode is the default). | MIT. Active. |
| [MS GraphRAG](https://github.com/microsoft/graphrag) | Entity graph + summaries. Expensive build. | Local and global search. | "Largely in maintenance mode." MIT. |
| [HippoRAG 2](https://arxiv.org/html/2502.14802) | Triples + synonym edges from vector similarity. | PageRank, started from vector results. Falls back to vectors. | Research code. |
| [Hindsight](https://github.com/vectorize-io/hindsight) | Facts, entities, time. AI on each save + background work. | Vector, BM25, graph, and time searches in parallel, then RRF. | Postgres + pgvector. MIT. |
| [Supermemory](https://github.com/supermemoryai/supermemory) | Facts that replace older facts ("moved to SF" replaces "lives in NYC"). | Hybrid. | Mostly hosted. |
| [A-MEM](https://github.com/agiresearch/A-mem) | Notes with links. AI writes the links. | Vectors (the same model as AIfred). | ChromaDB. MIT. |

Embedded graph engines for a Rust and SQLite app:

- Kuzu: archived on 2025-10-10 ([source](https://oneuptime.com/blog/post/2026-08-12-kuzu-archived-pin-0-11-3-fork-or-migrate/view)). Do not use.
- CozoDB: last release 0.7.6 on 2023-12-11 ([crates.io](https://crates.io/api/v1/crates/cozo)). Stalled.
- GraphQLite ([docs.rs](https://docs.rs/crate/graphqlite/0.5.0)) and sqlite-graph ([docs.rs](https://docs.rs/crate/sqlite-graph/0.1.0)): young SQLite graph extensions.
- Plain SQLite tables with `WITH RECURSIVE` are enough for walks of 1 to 3 steps at AIfred's size (thousands of rows). No new part is necessary.

## Benchmarks

All numbers below are self-reported by the project, unless the text says other.

mem0 paper, LoCoMo test, AI judge score ([arXiv 2504.19413](https://arxiv.org/html/2504.19413)):

| | Single-hop | Multi-hop | Open-domain | Temporal | Overall |
|---|---|---|---|---|---|
| All chat in context | | | | | 72.90 |
| mem0 with graph | 65.71 | 47.19 | 75.71 | 58.13 | 68.44 |
| mem0 with vectors only | 67.13 | 51.15 | 72.93 | 55.51 | 66.88 |

- The graph gave +1.6 overall. It helped time questions (+2.6). It hurt multi-step questions (−4.0).
- The graph doubled the stored memory and made the slowest searches about 1 s slower.

Zep paper, LongMemEval test, gpt-4o ([arXiv 2501.13956](https://arxiv.org/html/2501.13956v1)):

- All chat in context: 60.2%. Zep: 71.2%.
- Better: preferences (20% to 57%), time questions (45% to 62%), questions over many sessions (44% to 58%).
- Worse: questions about what the assistant said in one session (94.6% to 80.4%).

HippoRAG 2 paper ([arXiv 2502.14802](https://arxiv.org/html/2502.14802)):

- Plain vectors 57.0, GraphRAG 49.6, LightRAG 6.6, HippoRAG 2 59.8.
- Two graph systems did worse than plain vectors.
- Build cost for 11,600 passages: GraphRAG 115M AI input tokens. Vectors: no AI tokens.

Independent and critical sources:

- [RAG vs GraphRAG (arXiv 2502.11371)](https://arxiv.org/html/2502.11371v2): vectors are better for single facts and details. Graphs are better for multi-step questions. The two help each other.
- [When to use graphs in RAG (arXiv 2506.05690)](https://arxiv.org/abs/2506.05690v3): graph RAG often does worse than plain RAG on real tasks.
- [LongMemEval paper (arXiv 2410.10813)](https://arxiv.org/abs/2410.10813): cheap changes to the vector index help a lot. Examples: index extracted facts with the text, and add time cues to the question.
- [Letta](https://www.letta.com/blog/benchmarking-ai-agent-memory): an agent with plain file search tools got 74.0% on LoCoMo, more than mem0 with graph (68.5%). Self-reported.
- Zep and mem0 dispute each other's LoCoMo numbers ([Zep blog](https://www.getzep.com/blog/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)). Numbers from different papers cannot be compared.

What the graph helps with, in these sources:

- Questions about time and change: "where do I live now", "what changed since last month".
- Questions over many old chats.
- Lists of all things linked to one person or project.

What it does not help with: single facts and the "book my flight" → "window seats" kind of match. Vectors do that.

## Recommendation for AIfred

Keep word search and meaning search. Add three small parts. Each part uses data or AI calls that AIfred has already.

1. Entities and links.
   - New tables: `entity` (person, project, client, repo) and `entity_link` (entity ↔ item key, fact, or chat message).
   - Correction (2026-11-10): the saved GitHub and ClickUp items have no people. They have only the repository, the list, and the pull request links. Only email items and `waiting` have people. So most person links need an AI call or a change to the downloads.
   - Fill the rest in AI calls that run already: the item look-up that writes `item_context`, and the call that saves a fact.
2. Fact time.
   - Add `replaced_by` and `valid_until` to `memory`.
   - When the AI saves a fact, it gets the 3 nearest facts (meaning search). It can say "this replaces fact 12" in the same call.
   - Search skips replaced facts. The sidebar shows them under "Forgotten" or "Replaced".
3. Graph as a third list in the merge.
   - When a question names an entity, the items linked to it become a third result list.
   - `merge` (`db.rs:1195`) takes this list like the other two. RRF needs no other change.

Do not add: Graphify, Graphiti, LightRAG, GraphRAG, a graph database server, Kuzu, or CozoDB.

## Built on 2026-11-10: linked items in search results

The first part has no new table and no AI call. It uses the links that AIfred has already (`crates/aifred-core/src/related.rs`).

- The search tools for local tasks, GitHub items, and ClickUp tasks add `related` to each result.
- `related` has at most 8 items. Each item has its key, title, place, and the reason:
  - "parent" or "subtask": the subtask tree.
  - "same mission: <title>": an open mission with the same item.
  - "closes" or "closed by": a GitHub pull request and its issues.
- A linked GitHub issue that is not saved gets the title from the link. On 2026-11-10, 18 of the 24 linked issues were not saved.
- This is a graph walk of one step from each search result. Graphiti uses the same idea.
- Facts and chat messages have no item key, so they have no linked items.

To review later: ask AIfred about an item with a pull request, a subtask, or a mission. Then look at the `related` part of the search tool answer.

## Grouped facts

Question: can facts be grouped in a tree ("Company → Development → Workflow"), so the AI reads one group and does not search all facts?

How AIfred uses facts now (README, "Memory"):

- When all facts are 2,000 tokens or less, all facts go into every message. Grouping does not save anything then.
- When memory is larger, the "Always" facts and up to 8 matching facts go into each message.

What other systems do:

| System | Shape |
|---|---|
| [mem0](https://docs.mem0.ai/platform/features/custom-categories) | Flat facts. Each fact has several category labels. |
| [Memobase](https://raw.githubusercontent.com/memodb-io/memobase/main/readme.md) | 2 levels: topic and sub-topic. |
| [Letta](https://docs.letta.com/guides/agents/memory-blocks) | Named blocks that are always in the prompt. |
| [Claude memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool) | Folders and files. The AI looks at the folder list first. |
| [Claude Code](https://code.claude.com/docs/en/memory) | An index file with one line for each memory, and topic files that load when necessary. |
| [ChatGPT](https://help.openai.com/en/articles/8590148-memory-faq), [Gemini](https://support.google.com/gemini/answer/16413516) | Flat list. |
| [Supermemory](https://supermemory.ai/docs/user-profiles/buckets.md) | Up to 50 topic buckets. A memory can be in more than one. |

No system makes one strict path the only way to find a fact.

Research results (all self-reported):

- [RAPTOR](https://arxiv.org/html/2401.18059): a search over all nodes at once is better than a walk down the tree.
- [MemTree](https://arxiv.org/html/2410.14052v3): its search also treats the tree as one flat list. A walk down the tree was worse when it returned 5 results or fewer. On chat memory, the tree and the flat list were almost equal (84.8 and 84.4). Each new item cost 3.27 AI calls.
- In most tests, all memory in the prompt was best when it fitted.
- No study measured what happens when a fact is in the wrong group.

Recommendation:

1. Give each fact one group path of at most 2 levels, for example "Company / Development". Put deeper detail in the fact text: "QA lead: Cccc".
2. Keep word search and meaning search over all facts. Search finds a fact also when its group is wrong.
3. When memory is larger than 2,000 tokens, put a list of the groups with their fact counts into each message. This is the Claude Code index pattern.
4. Add a tool that reads one whole group.
5. The AI gives the group when it saves a fact. The user can change it in the sidebar. No question to the user.

## Built on 2026-11-10: tags on facts

The user chose tags instead of groups: a fact can be in more than one topic. Sending all facts while memory is small stays.

- Each fact has 0 to 5 lowercase tags (`memory.tags`, schema 32).
- The AI gives each new fact 1 to 3 tags. It reuses the tags from `memory_tags`.
- New AI tools: `memory_tags` (each tag and its fact count), `facts_with_tag`, and `tag_memory`.
- When memory is larger than 2,000 tokens, the instructions get the tag names. They do not get the counts: a count changes with each new fact, and then the cached instructions change.
- The sidebar shows the tags of each fact, a tag filter, and a "Tags" editor.

## Before building

- The current data folder is small (0 facts, 10 chat messages, 40 work items on 2026-11-10). This is too little to show a problem.
- Collect 20 real questions where AIfred gave a bad answer because of memory. Sort them: wrong old fact, missing link to a person or project, or bad meaning match.
- Build part 2 if "wrong old fact" is common. Build part 1 and 3 if "missing link" is common. Build nothing if the problems are bad meaning matches. For those, try a better embedding model first.

## Not verified

- The exact number of AI calls for each message in Graphiti and mem0. The papers do not give it.
- Whether LazyGraphRAG is in the open-source GraphRAG package.
