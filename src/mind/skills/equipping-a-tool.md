# Getting hold of a tool you don't have

When a job needs something the workshop hasn't got, **getting it is part of the job** — not a
blocker to report and not a reason to hand back a worse answer. Degrading to a web search when
the real answer was "install it" is the failure this note exists to prevent.

What follows is the order that works. It ends with writing a note, and the note is the point:
without it the next job re-solves the same problem from nothing.

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

## 5. Keep what only they can create somewhere durable

**A logged-in session is a credential.** For anything reached by driving an app that is already
signed in, being signed in *is* the whole of the access — and no note can rebuild it. Only the
person can sign in again.

So a browser profile, a session store, a device pairing, an auth cache: those live under
`{drive_dir}`, and the tool is pointed at them explicitly. Never under `bin/`.

A key or token is different: it goes in a secret file, and the note records **where it is and
what it opens** — never the value, which does not get pasted into a note, a log, or a message.

## 6. Actually make a real call

**Do not write the note yet.** Run the thing, on the real target, and get a real result back.

A tool that was installed and configured but never exercised is the most expensive kind of
false confidence: the note reads as settled, the next job trusts it, and the failure surfaces
somewhere far away with no clue it started here. Getting a `--version` to print is not a call.
Doing the smallest real version of the actual job is.

If it doesn't work, that is still this step — read the error, fix it, run it again. The
workshop only wants notes about things that worked.

## 7. Write the note

Now. In the workshop, under a filename that is the tool's name, with front matter:

    ---
    purpose: one line — what this does, specific enough that someone matching a job against it can tell
    use: the-command
    ---

Then the prose, which is where everything that matters goes: what it's for and when to reach
for something else, how to actually call it, the traps you hit, what a good result looks like,
and **how to get it back** if the command is missing on some future machine.

Two rules the note lives or dies by:

- **Mark what rots.** Flags, endpoints, prices, what a particular site looks like — say plainly
  which parts to re-check. A note trusted past its expiry is worse than no note, because it
  will be believed.
- **Write the entry point, never the argument list.** `use:` names the command; the command
  answers `--help` for itself. A flag list copied into a note is a second copy of something
  that changes without telling you.

Don't note the trivial or the one-off. A workshop you can't find anything in is no workshop.

## When it doesn't work out

Sometimes the answer is that this can't be done here — no API, a login wall you shouldn't
climb, a paid tier they haven't got. Say so plainly, say what it would take, and hand back
whatever partial result you did get. That is a real answer. Quietly substituting something
easier and reporting it as done is not.
