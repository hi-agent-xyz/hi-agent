// Client for the inbound file channel.
//
// Files are handed artifacts: POST multipart bytes to /api/in/file with the
// conversation header, and the backend stores them + wakes the mind on the file channel.

export interface UploadFailure {
  index: number;
  name: string;
  error: string;
}

export interface UploadResult {
  attempted: number;
  received: number;
  failed: UploadFailure[];
}

export class FileUploadError extends Error {
  readonly status: number;
  readonly result: UploadResult | null;

  constructor(message: string, status: number, result: UploadResult | null) {
    super(message);
    this.name = "FileUploadError";
    this.status = status;
    this.result = result;
  }
}

function uploadResult(value: unknown): UploadResult | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as {
    attempted?: unknown;
    received?: unknown;
    failed?: unknown;
  };
  if (typeof raw.received !== "number") return null;
  const failed = Array.isArray(raw.failed)
    ? raw.failed.flatMap((item) => {
        if (!item || typeof item !== "object") return [];
        const failure = item as {
          index?: unknown;
          name?: unknown;
          error?: unknown;
        };
        if (
          typeof failure.index !== "number" ||
          typeof failure.name !== "string" ||
          typeof failure.error !== "string"
        ) {
          return [];
        }
        return [
          {
            index: failure.index,
            name: failure.name,
            error: failure.error,
          },
        ];
      })
    : [];
  return {
    attempted:
      typeof raw.attempted === "number" ? raw.attempted : raw.received + failed.length,
    received: raw.received,
    failed,
  };
}

function parseJson(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

/** Send one or more files and return the backend's per-batch outcome. */
export async function postInFiles(opts: {
  files: File[];
  signal?: AbortSignal;
}): Promise<UploadResult> {
  const fd = new FormData();
  for (const file of opts.files) {
    fd.append("file", file, file.name || "file");
  }

  const res = await fetch("/api/in/file", {
    method: "POST",
        body: fd,
    signal: opts.signal,
  });

  const responseText = await res.text().catch(() => "");
  const contentType = res.headers.get("content-type") ?? "";
  const result =
    contentType.includes("application/json") && responseText
      ? uploadResult(parseJson(responseText))
      : null;

  if (!res.ok || (result?.failed.length ?? 0) > 0) {
    const detail =
      result === null ? responseText : result.failed[0]?.error ?? "";
    throw new FileUploadError(
      `/api/in/file POST failed: ${res.status} ${res.statusText}${detail ? ` - ${detail}` : ""}`,
      res.status,
      result,
    );
  }

  return (
    result ?? {
      attempted: opts.files.length,
      received: opts.files.length,
      failed: [],
    }
  );
}
