# Home: a semantic work tree

Home answers what Hi Agent and the person are discussing, what work is in hand,
and what changed. It is a read projection of existing records, not another task
ledger or session lifecycle store. It remains a normal factory view.

## Nodes and relationships

`HomeNode` has a stable identity, a semantic kind, a title, typed data and source
references. Kinds are `core`, `topic`, `task`, `activity`, `overview`, and `artifact`.
`HomeEdge` carries `contains`, `works-on`, `produces`, or `explains`, plus whether it
is a primary edge. Every non-root node has one primary parent and reaches `core`.
Additional references do not duplicate the destination. The primary tree is acyclic.

Nodes may have children at any depth. Collapse, selection, zoom and coordinates
belong to the window, never to a task or a session. Overview nodes can be embedded
in the core instead of rendered as peripheral cards. Node identity does not imply
a particular visual container.

## Internal mapping

- A task is `task:<subject>` from the task ledger. Its status and lifecycle timestamps
  stay authoritative even when its worker is idle, fails a turn, or disappears.
- An activity is `session:<run>:<session>`. The run is required because session slugs
  can recur after restart. Online records come from the registry; ended records come
  from the durable session index. An online record wins a read-boundary overlap.
- Reaction, Cognition and Reflection sessions compose the single core. Other sessions
  remain visible activities regardless of owner, role specialization, or task binding.
- A session's `subject` joins it to a task. Without a resolvable task, it connects to
  the core. Its technical `owner` remains inspectable but does not determine semantic
  placement or create a new task grouping.
- Explicit project/system metadata supplies topic membership. Similar wording is not
  evidence of ownership. Multiple tags can reference one task, with one primary parent.
- Result references resolve existing files/views. A result may be a child node with
  its own children; a thumbnail is optional and never decides whether work is shown.

`running`, `waiting` and `idle` are registry states of a live session. In particular,
`waiting` means queued work, not a request for the person to answer. Last-turn outcome
and session termination are separate facts. A retained tool action describes current
activity only while its session is running.

## Core overview

The core presents shared context, public direction, significant updates and decisions
needed from the person. Each statement has source references, an update time, freshness
and links to related work. It does not expose private reasoning or interpret a busy
flag as a plan. Internal output tails are not public statements.

User-visible conversation excerpts and factual task transitions are valid grounded
overview content. A previous response is earlier context after a new user message.
A direction or decision request requires an explicitly published, source-backed
statement; it must not be inferred by copying a worker's internal narrative.

## Retention and completeness

All `todo`, `doing` and `serving` tasks remain present. `done` and `cancelled` tasks
remain for 24 hours from closure, without a count cap or an artifact requirement.
Their visual emphasis decreases with age, not their readability. An expired task
needed by a retained activity stays as context without being relabeled open.

All online sessions remain present. Ended sessions follow the same 24-hour window.
Unknown closure times are shown as unknown and retained rather than fabricated.
The time-window history read uses durable records, not the 500-row recovery cache.
Missing sources are distinct from empty sources; refresh failure keeps the last
successful snapshot with a visible stale-source indication.

Search includes collapsed nodes and reveals the ancestor chain of a result. Collapse
is not exclusion, and the graph is not forcibly shrunk to unreadability to fit a screen.
Narrow views use a connected, recursively expandable flow of the same nodes and edges.
