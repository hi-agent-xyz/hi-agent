// Dev-seam backstop for `<html lang>`.
//
// **The server is authoritative.** In prod, `appearance::index()` rewrites
// `<html lang="…">` to the stored setting as it serves the page, and that is the
// mechanism — one read, at boot, no request from the client. See `appearance/mod.rs`.
//
// This exists because that injection does *not* run in dev: Vite serves the page itself,
// so the Rust `index()` never sees it and the built-in views would resolve their copy
// against `lang="en"` no matter what the setting said. That is the same dev/prod
// asymmetry CLAUDE.md warns about for the import map, and this is the cheaper half of the
// same fix — a Vite plugin mirroring the injection would be the tidier one.
//
// So: same value, same source of truth (the setting), just fetched over HTTP because in
// dev nothing stamped it into the document. In prod this agrees with what the server
// already wrote and changes nothing.
//
// Why the attribute at all, rather than a hook or a prop: a bundled view resolves its
// strings at module scope (`const L = words()`), and views load as bare ESM through the
// import map — there is no React context in scope at that moment and no host prop to
// thread. `document.documentElement.lang` is already global, already there, and already
// the platform's own answer to this question, the same way the theme uses `data-theme`.

/** The stored language value (`system` / `en` / `zh-Hans`), or null if unreadable. */
async function storedLanguage(): Promise<string | null> {
  try {
    const r = await fetch("/api/settings");
    if (!r.ok) return null;
    const s = (await r.json()) as { appearance?: { language?: { value?: string } } };
    return s.appearance?.language?.value ?? null;
  } catch {
    return null;
  }
}

/**
 * Publish the language setting on `<html lang>` if it isn't already there.
 *
 * Safe to call more than once. A value that cannot be read leaves the document's existing
 * `lang` alone rather than blanking it, so a failed settings call degrades to whatever the
 * server (or the build) put there instead of to nothing.
 *
 * Timing: this is async, and a view that read `lang` before it resolved would fall back to
 * the OS locale. In prod that window does not matter — the server already stamped the
 * right value into the HTML. In dev it is a real if narrow window, and views arrive much
 * later than boot (the agent shows one mid-conversation), so in practice it is closed.
 */
export async function applyLanguage(): Promise<void> {
  const value = await storedLanguage();
  if (value && document.documentElement.lang !== value) {
    document.documentElement.lang = value;
  }
}
