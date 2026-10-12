# Work Membership

Home is a projection of work relations, not the place where project facts originate.
This supersedes the claim in `home.md` that every grouping is surface-only. A category
and its order remain surface preferences; task membership is durable work data.

## Records

An existing `memory/facets/projects/<subject>/facet.md` is the project identity. Its
subject is stable; changing its display label does not rename the reference. This change
does not create a second project registry or infer projects from `systems`.

Each task has a JSON-encoded `work` frontmatter field, written in the same atomic record
as its creation line:

```json
{
  "assignment": {
    "state": "project",
    "project": "open-badminton",
    "continues": "open-badminton-site-v1"
  },
  "source": "user",
  "basis": "The user said this is a feature of the website.",
  "revision": 1
}
```

Other assignments are `{"state":"independent"}` and `{"state":"pending"}`. Both
require a basis. Absent or invalid work on an old record is unresolved legacy data, not
independent work. Invalid stored bytes are preserved during unrelated task updates.

`continues` is optional. It names an existing task with the same confirmed project, and
must be acyclic. It expresses continuation, not dependency or parent-child display rank.
A parent cannot change projects while another task still continues it; resolve those
links explicitly first. Membership does not merge promises or task records.

## Writers And Consistency

- `hi_task_open` requires a work input. Unknown projects or invalid continuation links
  fail before the task is published. There is no second asynchronous classification call.
- Cognition owns `hi_task_relate`, because it witnessed the request. Workers report
  contradictory evidence to it rather than rewrite task files.
- Corrections compare `expected_revision` against the current task under the task write
  lock. Revision 0 is only for legacy rows. A stale writer must reread, not overwrite.
- Source `user` is reserved for explicit user statements or corrections. Context
  judgments cannot displace user assignments or remove their protection. The direct
  task-detail UI sets source to user on the server, not from client authority claims.
- Every correction adds a store-owned `related` timeline event including the previous
  relation. That event does not supersede a waiting line or become work progress.
- The active-task projection and worker brief carry the membership. The brief also
  includes the referenced project record, so execution does not reconstruct it from titles.

These are guarantees for structured store verbs. As with task status, unsandboxed agents
can edit files; prompts prohibit bypassing the verbs, but this is not a filesystem security
boundary. Source evidence is auditable prose, not mechanically proven attribution.

## Display Projection

Home groups optionally carry `project: <subject>`. This is a display anchor, not membership.
An anchor must reference an existing project and be unique; project anchors cannot nest
inside other projects. It can be stored before any
tasks exist. Its rendered node ID is `project:<subject>`, independent of its label.

Confirmed tasks enter that anchor immediately; without one, Home creates a project heading
at the core. A stale membership list elsewhere cannot divert them. Existing subgroups
inside their project's explicit anchor may organize tasks into display worklines. A
pending task enters a visibly pending branch; independent tasks stay independent.
Legacy records retain old placement and retention with an unrecorded-membership label.
They are never promoted to project facts by reading a group, title, system, or keyword.

`members` order is honored independently of API sorting. Shared result entities have a
separate display node per task usage, preserving the entity ref and correct task handoff.
The viewport's cut retains ancestors with an offered child, including activity cards.
Hidden execution cards are counted on their task rather than admitted as invisible orphans.

Project membership is not a retention proxy: one open task does not keep every finished
project task on Home. Confirmed finished tasks follow the notice policy. This first change
does not introduce dependency-based retention or a general knowledge graph.

## Finding Closed Content

`GET /api/projects` lists existing project subjects. `GET /api/projects/<subject>/tasks`
lists all confirmed member tasks, including closed ones, independent of Home retention.
Home exposes this directory even when a finished project has no current chart node. A
focused project exposes its own content entry; task and attached-result names are searchable,
and each result keeps its native view or task-line handoff. The task detail exposes membership,
evidence, and an explicit correction control with a project picker.

The project browser uses the same capped row attachments as the ledger. Full task history
and older attachments remain in task details; it is not an unlimited artifact inventory.

## Migration And Delivery

No automatic semantic migration occurs on boot. Review legacy tasks against the request
and project record, record each basis, and write expected-revision corrections. Do not
convert every category or `systems` value into a project. Migrate active work first; migrate
closed content when its project evidence is known. The old groups remain readable throughout.

For controlled offline migrations, `examples/work-membership.rs` accepts a reviewed JSON
manifest and defaults to a dry run. Applying requires `--apply`; stop the core before using
it against its data directory, because the task lock is process-local. A bulk migration is
per-task CAS, not a cross-file transaction; earlier successful changes remain if a later
entry fails, and the output states which revisions landed. Back up before applying.

`docs/arch/examples/open-badminton-membership.json` contains four reviewed legacy tasks,
with evidence per task and no guessed continuation links. It is not applied automatically:

```sh
cargo run --example work-membership -- data docs/arch/examples/open-badminton-membership.json
# After stopping the core and backing up, use the same command with --apply.
```

This implementation covers durable membership, propagation, corrections, projection and
project discovery. It does not automatically rewrite existing instance data, deploy/restart
the running core, introduce workline identities, or pin every category's canvas position.

## Acceptance

- A website follow-up is under its project on its first Home projection.
- A stale Home arrangement cannot override confirmed work membership.
- Renaming a project heading preserves its identity; recategorizing preserves membership.
- Pending is distinguishable from explicitly independent work.
- A stale correction loses its CAS; ordinary cleanup cannot erase a user correction.
- Continuation cycles, dangling subjects and different-project continuations are refused.
- A relation correction does not clear a waiting state.
- Shared results remain accessible from every task usage.
- Every drawn non-root node retains a path to the current root after cutting.
- Closed project work remains discoverable without resurrecting it on the current chart.
