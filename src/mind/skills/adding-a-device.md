# Adding a device

*A starting point, not gospel. Lines marked **[perishable]** go stale — products,
prices, permission names and APIs all move. Re-check those before you lean on them;
the shape of the thing below doesn't rot. Checked 2026-07-27.*

Sometimes the work isn't where I am. Something has to happen on the Mac in the other
room, or on a phone, or on a machine that has the account already signed in. Giving
myself hands and eyes over there is the whole problem, and there are a few ways to do
it — none free, all needing the person to do something once.

## A machine they own, over SSH

Try this first. I already have a shell, so nothing new is needed: they add my key (or
I use one already there), and from then on it's files, builds, tests, servers, logs,
ffmpeg, whatever runs headless.

The trap — and this one cost us real time — is that **a process started over SSH has
no window server and no granted permissions**. Screenshots, keystroke injection,
`osascript`, anything that touches the screen: it fails, or worse, silently no-ops
and reports success. Identical code driven from that machine's own logged-in desktop
session works fine. So SSH is not a way to operate an app's UI. If the job needs the
screen, something must already be running *inside* the desktop session there.

## A phone, over adb

For an Android handset: `adb shell screencap`, `adb shell input tap/text/swipe`,
`adb pull` for files. Enough to see and act, one step at a time. [perishable — command
details and what each Android version allows]

The costs are honest ones: developer mode has to be turned on by the person, it wants
USB or a pairing dance on the same network, and it isn't reachable from elsewhere.
Fine for a phone on the desk; poor for a phone in a drawer across town.

## abacad — https://abacad.ai [perishable — everything in this section]

Purpose-built for exactly this: *"connect a phone, laptop, or browser as a device —
then point your coding agent at one endpoint and let it drive, with you approving
every step."* Their words. One MCP endpoint; commands route to a device by its id, so
several devices sit behind one connection, and the device dials out — no port to open.
It offers screen viewing, reading on-screen text, injecting input, file transfer,
screen recording, and a jump host for ssh/scp to the machine. Their framing is that
the agent picks the right rung per action — API, shell, accessibility tree, or
screenshot.

Clients as of the check date: macOS 14+ (grant Accessibility and Screen Recording
once, then it lives in the menu bar) and Android 11+ (one accessibility permission
covers seeing and acting, and it survives reboots). Windows 11 and Linux x86_64 are
listed as in development. There is also a browser device with nothing to install —
open its link in a tab and that tab is the device.

Setup is on the person: an account, a device credential from their console, then scan
the QR or paste the connection URL into the app. The credential is shown once.

The trade-off to say out loud: traffic relays through their servers, which is what
makes it work from anywhere and also what makes it a party to trust. Their own advice
is not to leave a permanently connected device signed into sensitive accounts —
banking, email. I'd take that seriously. No pricing was stated on the site; ask them
or check before promising anything about cost.

## Where there's no API, the logged-in session *is* the credential

Plenty of platforms have no open API worth using. Then the way in is the app that's
already signed in on that device — I drive it like a person would. There's no key to
store and nothing to leak, which is a feature. It also means the account's own limits
apply, and that operating someone's account is something to be plain about, not quiet
about.

## When it's genuinely blocked, hand it back

A captcha, a login wall, a code sent to their phone — that's a stop, not a puzzle. I
say exactly which step I'm at and what I need, in the channel they're actually on, and
wait. I don't try to defeat it and I don't silently retry.

## After it works

Write down what's true about that device — how I reach it, what the OS actually
permits there, what it's signed into — so the next job starts from that instead of
rediscovering it. Reachability and grants are properties of the environment, and they
change without telling me: re-verify with a real call, not from memory.
