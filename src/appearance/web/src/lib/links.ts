export interface SpeechTextPart {
  kind: "text";
  text: string;
}

export interface SpeechLinkPart {
  kind: "link";
  text: string;
  href: string;
  label: string;
}

export type SpeechPart = SpeechTextPart | SpeechLinkPart;

// Where a URL ends. Latin prose puts a space there; CJK prose puts nothing —
// `…/sample.mp3。听完再告诉我` has only the punctuation to stop on, and a greedy
// class swallows the rest of the sentence into the href. So CJK punctuation,
// ideographs, kana, hangul and full-width forms terminate a URL too, alongside
// whitespace and the markup quotes.
const URL_STOP =
  "\\s<>\"'`\\u2013\\u2014\\u2018-\\u201F\\u2026\\u3000-\\u303F\\u3040-\\u30FF\\u3400-\\u4DBF\\u4E00-\\u9FFF\\uAC00-\\uD7AF\\uFF00-\\uFFEF";
const URL_RE = new RegExp(`https?://[^${URL_STOP}]+`, "gi");
// ASCII sentence punctuation the class above cannot exclude (`.` and `)` are
// ordinary URL characters), stripped only where it trails the whole match.
const TRAILING_PUNCTUATION_RE = /[.,!?;:)\]}]+$/u;

function appendText(parts: SpeechPart[], text: string): void {
  if (!text) return;
  const previous = parts[parts.length - 1];
  if (previous?.kind === "text") {
    previous.text += text;
  } else {
    parts.push({ kind: "text", text });
  }
}

function linkLabel(url: URL): string {
  return url.hostname.replace(/^www\./i, "") || url.href;
}

/** Split plain speech into text and safe, compact http(s) links. */
export function splitSpeechLinks(text: string): SpeechPart[] {
  const parts: SpeechPart[] = [];
  const re = new RegExp(URL_RE.source, URL_RE.flags);
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(text)) !== null) {
    appendText(parts, text.slice(cursor, match.index));

    const candidate = match[0];
    const punctuation = candidate.match(TRAILING_PUNCTUATION_RE)?.[0] ?? "";
    const href = punctuation ? candidate.slice(0, -punctuation.length) : candidate;

    try {
      const url = new URL(href);
      if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("unsupported protocol");
      parts.push({ kind: "link", text: href, href: url.href, label: linkLabel(url) });
      appendText(parts, punctuation);
    } catch {
      appendText(parts, candidate);
    }

    cursor = match.index + candidate.length;
  }

  appendText(parts, text.slice(cursor));
  return parts;
}
