export interface TransferFileItem {
  kind: string;
  getAsFile(): File | null;
}

export interface TransferData {
  types: ArrayLike<string>;
  items: ArrayLike<TransferFileItem>;
  files: ArrayLike<File>;
}

/** True when a drag payload contains files, without claiming text or URL drags. */
export function transferHasFiles(
  data: Pick<TransferData, "types" | "items">,
): boolean {
  if (Array.from(data.types).includes("Files")) return true;
  return Array.from(data.items).some((item) => item.kind === "file");
}

/** Clipboard/drag files, preferring item flavors and falling back to FileList. */
export function filesFromTransfer(
  data: Pick<TransferData, "items" | "files">,
): File[] {
  const files = Array.from(data.items)
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
  return files.length > 0 ? files : Array.from(data.files);
}

/** A view-owned editor keeps its native keyboard and clipboard behavior. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return (
    target.closest(
      "input, textarea, select, [contenteditable]:not([contenteditable='false'])",
    ) !== null
  );
}

/** The host's own text line: text pastes stay native, file pastes become handoffs. */
export function isBaseTextInputTarget(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("[data-hi-base-text-input]") !== null;
}
