import { describe, expect, it } from "vitest";
import { attachmentOf, clock, pictureSource } from "./attachments";

describe("attachmentOf", () => {
  it("plays a clip the agent handed over from the route that knows where its playable bytes are", () => {
    expect(attachmentOf("att:3f9a0c11d2e4b5a6", "video/mp4")).toEqual({
      ref: "att:3f9a0c11d2e4b5a6",
      kind: "clip",
      url: "/api/attachments/3f9a0c11d2e4b5a6/playable",
      preview: "/api/attachments/3f9a0c11d2e4b5a6/preview.v1",
    });
  });

  it("shows a picture the agent handed over as it is", () => {
    expect(attachmentOf("att:3f9a0c11d2e4b5a6", "image/png").url).toBe("/api/attachments/3f9a0c11d2e4b5a6");
  });

  it("reads a file the person handed over from the media route, as it always was", () => {
    const photo = attachmentOf("file/2026-09-22/14/03-22.jpg", "image/jpeg");
    expect(photo.kind).toBe("picture");
    expect(photo.url).toBe("/api/media/file/2026-09-22/14/03-22.jpg");
    expect(photo.preview).toBe(photo.url);
    expect(attachmentOf("file/2026-09-22/14/03-22.pdf", "application/pdf").kind).toBe("file");
  });
});

describe("clock", () => {
  it("says a length the way a player does", () => {
    expect(clock(30_000)).toBe("0:30");
    expect(clock(724_000)).toBe("12:04");
    expect(clock(3_729_000)).toBe("1:02:09");
  });
});

describe("pictureSource", () => {
  const figure = {
    ref: "att:3f9a0c11d2e4b5a6",
    kind: "picture",
    url: "/api/attachments/3f9a0c11d2e4b5a6",
    preview: "/api/attachments/3f9a0c11d2e4b5a6/preview.v1",
    width: 3840,
    height: 2160,
  };

  it("draws a figure in a small box from its preview, however large the original is", () => {
    expect(pictureSource(figure, 320, 2)).toBe(figure.preview);
    expect(pictureSource(figure, 960, 1)).toBe(figure.preview);
  });

  it("loads the original once the box is wider than the preview is sharp", () => {
    expect(pictureSource(figure, 960, 2)).toBe(figure.url);
    expect(pictureSource(figure, 1600, 1)).toBe(figure.url);
  });

  it("measures the preview by the side the picture's shape fits it by", () => {
    // A tall figure is fitted by its height: 540 tall is 270 wide.
    const tall = { ...figure, width: 1000, height: 2000 };
    expect(pictureSource(tall, 270, 1)).toBe(figure.preview);
    expect(pictureSource(tall, 271, 1)).toBe(figure.url);
  });

  it("uses the original when nothing says how large the preview is", () => {
    expect(pictureSource({ ...figure, width: null }, 10, 1)).toBe(figure.url);
  });
});
