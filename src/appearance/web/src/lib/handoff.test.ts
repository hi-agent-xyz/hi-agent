import { describe, expect, it } from "vitest";

import {
  filesFromTransfer,
  transferHasFiles,
  type TransferData,
  type TransferFileItem,
} from "./handoff";

const file = (name: string) => ({ name }) as File;

function item(value: File | null, kind = "file"): TransferFileItem {
  return {
    kind,
    getAsFile: () => value,
  };
}

function transfer(
  items: TransferFileItem[],
  files: File[] = [],
  types: string[] = [],
): TransferData {
  return { items, files, types };
}

describe("transferHasFiles", () => {
  it("recognizes the Files flavor before item details are available", () => {
    expect(transferHasFiles(transfer([], [], ["Files"]))).toBe(true);
  });

  it("recognizes a file item and ignores text-only drags", () => {
    expect(transferHasFiles(transfer([item(file("a.txt"))]))).toBe(true);
    expect(transferHasFiles(transfer([item(null, "string")], [], ["text/plain"]))).toBe(false);
  });
});

describe("filesFromTransfer", () => {
  it("uses concrete item flavors and drops null item conversions", () => {
    const a = file("a.txt");
    const b = file("b.pdf");
    expect(filesFromTransfer(transfer([item(a), item(null), item(b)]))).toEqual([a, b]);
  });

  it("falls back to the FileList-compatible collection", () => {
    const fallback = file("clipboard.png");
    expect(filesFromTransfer(transfer([], [fallback]))).toEqual([fallback]);
  });
});
