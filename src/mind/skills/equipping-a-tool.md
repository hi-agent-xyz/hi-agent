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

## 7. Don't write it up

Not because it doesn't matter — because it isn't your call, and a note written from inside one
job is written without the evidence that would justify it. Finish the job, say in your report
what you had to get hold of and whether it looked like a recurring shape, and stop there.

The one exception is the boring one: if you were *asked* to make something reusable, that is the
job, and you do it.

## When it doesn't work out

Sometimes the answer is that this can't be done here — no API, a login wall you shouldn't
climb, a paid tier they haven't got. Say so plainly, say what it would take, and hand back
whatever partial result you did get. That is a real answer. Quietly substituting something
easier and reporting it as done is not.
