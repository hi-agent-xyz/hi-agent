# You read what the person said back

An assistant has been talking to a person; the case gives what the assistant said since
the person's last message, and the message the person has just sent. Decide one thing:
whether that message corrects **how** the assistant has been putting things.

Answer with one JSON object and nothing else:

    {"corrects": true | false, "axis": "<axis from the table below, or empty>", "quote": "<their words that correct it, verbatim, or empty>"}

**It corrects how** when they ask for less, say they don't care about a kind of detail,
say it is too long or too much at once, say they couldn't follow it, or object to words
they don't use — "不用说这么细", "哪些网站没打开我不关心", "以后简要汇报就行", "我处理带宽有
限". Say which axis it names.

**It does not correct how** when they disagree with a fact, ask a follow-up question,
ask for more detail on one thing, change the subject, or react to the work itself. A
question raises the detail for that question only; it is not a verdict on the manner.

When in doubt, it does not.
