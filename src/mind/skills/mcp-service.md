---
purpose: reach a service that only speaks MCP — list what it offers, read one tool's schema, call it
use: hi mcp
---

# Reaching an MCP service

Some services publish their capability as an MCP server and nothing else. `hi mcp` makes one
of those an ordinary command, so you can use it inside a job you're already running without
anything being loaded up front.

    hi mcp <endpoint> list
    hi mcp <endpoint> schema <tool>
    hi mcp <endpoint> call <tool> '<json>'

`<endpoint>` is either a URL — `https://example.com/mcp` — or, **quoted as one argument**, the
command that starts a server: `"npx -y @modelcontextprotocol/server-everything"`. You don't have
to say which; a URL is recognised as one. Quote the command form, or its own flags get read as
`hi`'s.

## How to use it

**Always `list` first, then `schema` the tool you want.** Argument shapes come from the server
at the moment you ask, and they change without telling anyone. Never write a tool's arguments
down in a note and never work from memory of them — that copy is the thing that goes stale and
then fails in a way nobody traces back here.

    hi mcp https://example.com/mcp list
    hi mcp https://example.com/mcp schema search
    hi mcp https://example.com/mcp call search '{"query":"kyoto flights","limit":5}'
    hi mcp "npx -y @modelcontextprotocol/server-everything" list

Text results print as text, so a pipeline can read them. A tool that reports failure exits
non-zero, so `&&` and `set -e` behave the way you'd expect.

## What it can't do

**One call in, one answer out.** MCP servers can also stream progress and ask the client
questions mid-call; none of that survives a one-shot command. For nearly everything that
doesn't matter. When a service's whole value *is* the live stream, this is the wrong shape and
the honest answer is to say so rather than to poll it into looking like one.

## Traps

- **A server that needs a credential** takes it however that server takes it — usually an
  environment variable on the spawn, or a header. Keep the value in a secret file and have the
  command read it at the moment it runs; it doesn't go in this note, in a message, or in a log.
- **Spawning a server costs a process start every call.** For a handful of calls that's fine.
  If you're about to make dozens, that's the signal to write yourself a small script that opens
  one connection and does the batch — and to leave a note about it.
- **A wedged server fails after two minutes** rather than hanging your job forever. If you see
  that timeout, the server is the suspect, not your arguments.

## Perishable

- **What any given server offers.** Tool names, their arguments, and whether it wants a
  credential are all facts about that server this month. `list` and `schema` are the truth.
- **Endpoints and package names** for third-party servers move. Re-check before assuming one
  that worked before still exists.
