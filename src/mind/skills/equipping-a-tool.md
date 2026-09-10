---
purpose: getting hold of something a job needs and you haven't got — what to look for, what to ask the person for, where to keep it, and why not to write it up
---

# Getting hold of what a job needs

When a job needs something you haven't got, **getting it is part of the job**. Handing back a
worse answer while implying it is the answer is the failure this exists to prevent.

**This is not about building a tool.** Doing today's job and creating something reusable are
different acts, and the second is expensive in a way that is easy to miss: it costs a session to
build, it costs reading and trusting a note every time it is used, and once listed it costs a
line in the window of *every* session from then on — including all the ones about something
else. A capability used twice is worse than none.

So: do the job the simplest way that works, and keep what you build to what this job needs.
Whether a shape recurs often enough to be worth a real tool is decided later, by the part of the
agent that reads across days — it can see the four other times, and you can't. If it smells like
a recurring shape, say so in your report and leave it there.

## 1. Look first

    grep -rn "^purpose:" {skills_dir}

You may already have something. A tool built for a neighbouring job often covers this one, and
a second tool that overlaps an existing one makes the workshop harder to search for everybody
who comes after.

## 2. Work out what to get

Research it properly — what people actually use for this, not the first result. Prefer, in this
order:

- **Something already on this machine.** Check before installing: `command -v <thing>`.
- **A command-line tool.** It tells you its own arguments, it works inside a job already
  running, and you can write one yourself.
- **A small script of your own** over a dependency, when the job is narrow.
- **A service that speaks only MCP**, when that is the only way it is offered — a remote
  device, a vendor's hosted tools. Reaching one is § 4b below.

Keep it boring. A widely-used tool with a stable interface beats a clever one you'd have to
re-learn.

## 3. Ask for the one thing only they can do

Some steps are not yours: an account, an API key, a payment, a permission clicked on their own
machine. For those — **ask once, concretely, for that one thing.**

Say exactly what you need and where, in one message. Not "I need access to X" but the actual
step: the page to open, the button, the value to paste back. Don't stack up questions, don't
ask again while you wait, and don't ask for what you could find out yourself.

**Then carry on with everything that doesn't depend on the answer.** A job parked waiting is
the most expensive thing you can do.

## 4. Install it

Use the machine's own package manager where there is one — that puts the command on the PATH,
which is all a note needs. Nothing has to be copied into `bin/`.

`{bin_dir}` is for the two things a package manager can't give you: **scripts you wrote
yourself**, and **a wrapper that binds a stable name** to something whose real path differs per
machine. Anything in there needs the executable bit (`chmod +x`) and should answer `--help`,
because that is where the next session will look for its arguments.

Two things not to do:

- **Don't shadow a command that already exists.** A script of yours named `curl` or `python`
  on that PATH changes what every later session gets when it types that name.
- **Don't put anything in `bin/` you can't recreate.** It is disposable, and one day it will be
  deleted. Which brings us to the thing that is genuinely fragile:

### 4b. Registering an MCP server

Some services publish their capability as an MCP server and nothing else. **Registering one is
writing a skill** — an ordinary note in `{skills_dir}` whose front matter carries the server
object, exactly as that server's own documentation gives it to you:

    ---
    purpose: drive the handset in the study — read the screen, tap, type
    mcp: {"url":"https://example.com/mcp","headers":{"Authorization":"Bearer abd_key_…"}}
    ---

    What it's for, what it's connected to, and anything you had to learn the hard way.

That file is the whole registration, and the name comes from the filename — that name is what
gets asked for later. **Paste the object; don't rewrite it.** It is the same one every MCP client
takes, so whatever the server's docs show works here: several headers, a spawned server's
`env`, whatever else it wants. A spawned server is the other shape of the same object —
`mcp: {"command":"npx","args":["-y","@scope/pkg"],"env":{"TOKEN":"…"}}` — and you do not have to
say which transport it is, because `url` versus `command` already does.

It goes on **one line**, and it has to be a JSON object. If it is not, the server is skipped with
a warning and the errand that named it will simply find no such tool — so check it parses.

**You will not be the one to use it.** A session's tools are fixed when its thread opens, so a
server you register now cannot appear in your own toolset — there is no call you can make to try
it. Say so in your report: name the server, say it is registered and **not yet exercised**, and
say what you would have run first. Whoever dispatches the next errand names it on that errand,
and the first real call happens there.

That is a real gap in this procedure and not a formality — § 6 below exists because installed and
never exercised is the most expensive kind of false confidence, and here you genuinely cannot
close it yourself. So do the half you can: get the endpoint and the credential right, and be
precise about what is untested rather than writing the note as though it were settled.

## 5. Keep what only they can create somewhere durable

**A logged-in session is a credential.** For anything reached by driving an app that is already
signed in, being signed in *is* the whole of the access — and no note can rebuild it. Only the
person can sign in again.

So a browser profile, a session store, a device pairing, an auth cache: those live under
`{drive_dir}`, and the tool is pointed at them explicitly. Never under `bin/`.

A key or token is different: it goes in a secret file, and the note records **where it is and
what it opens** — never the value, which does not get pasted into a note, a log, or a message.

**One named exception: an MCP server's own object (§ 4b).** That line is config the host
parses, not prose anyone reads, and the rung doing the job never opens the file — the host
resolves it and attaches the server. Keeping the key out of it was tried and cost a verb, a
table and a registration split across two places, to defend against nobody: the person and this
host are both trusted, and the key arrived by being pasted into a channel that keeps every
message. So the object goes in whole. Everything else in this section stands — including that
the value still does not belong in a log, a message, or your report.

## 6. Actually make a real call

**Do not write the note yet.** Run the thing, on the real target, and get a real result back.

A tool that was installed and configured but never exercised is the most expensive kind of
false confidence: the note reads as settled, the next job trusts it, and the failure surfaces
somewhere far away with no clue it started here. Getting a `--version` to print is not a call.
Doing the smallest real version of the actual job is.

If it doesn't work, that is still this step — read the error, fix it, run it again. The
workshop only wants notes about things that worked.

## 7. Don't write it up

Not because it doesn't matter — because it isn't your call, and a note written from inside one
job is written without the evidence that would justify it. Finish the job, say in your report
what you had to get hold of and whether it looked like a recurring shape, and stop there.

The one exception is the boring one: if you were *asked* to make something reusable, that is the
job, and you do it. **Registering an MCP server (§ 4b) is that case and not this one** — the
skill there is not a write-up of how a job went, it is the mechanism itself, and the person
handing over a server has already decided it is worth having. Write it, and keep it to what the
server is and how to reach it.

## When it doesn't work out

Sometimes the answer is that this can't be done here — no API, a login wall you shouldn't
climb, a paid tier they haven't got. Say so plainly, say what it would take, and hand back
whatever partial result you did get. That is a real answer. Quietly substituting something
easier and reporting it as done is not.
