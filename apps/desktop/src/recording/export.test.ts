import { describe, expect, it } from "vitest";
import type { recording } from "@keyjutsu/types";
import {
  escapeHtml,
  frameTimes,
  guideHtml,
  guideMarkdown,
  stepFileBase,
  videoFormat,
} from "./export";

const exported: recording.Export = {
  title: "Put a cat in a PDF",
  recording: { width: 80, height: 24, title: "t", events: [] },
  cast: "",
  redactions: ["GitHub token ×1"],
  steps: [
    {
      step: "make-pdf",
      title: "Print the photo to a PDF",
      objective: "Cat.pdf exists on the desktop.",
      reason: "The desktop is in OneDrive.",
      commands: ["Start-Process cat.jpg -Verb Print"],
      printed: "PS> Start-Process cat.jpg -Verb Print\n",
      succeeded: true,
    },
    {
      step: "open",
      title: "Open it <now>",
      objective: "",
      commands: ["Write-Output '```'"],
      printed: "",
      succeeded: false,
    },
  ],
};

describe("export", () => {
  it("prefers MP4, then WebM, and says when it can do neither", () => {
    expect(videoFormat(() => true)?.extension).toBe("mp4");
    expect(videoFormat((m) => m.startsWith("video/webm"))).toEqual({
      mime: "video/webm;codecs=vp9",
      extension: "webm",
    });
    expect(videoFormat(() => false)).toBeNull();
  });

  it("names step files in order, safely", () => {
    expect(stepFileBase(3, "open-pdf")).toBe("03-open-pdf");
    expect(stepFileBase(12, "a b/c")).toBe("12-a-b-c");
  });

  it("draws a frame at each tick and at the end", () => {
    expect(frameTimes(0.25, 10)).toEqual([0, 0.1, 0.2, 0.25]);
    expect(frameTimes(0, 10)).toEqual([0]);
  });

  it("writes the guide as Markdown, a step at a time", () => {
    const md = guideMarkdown(exported, new Map([["make-pdf", "01-make-pdf.png"]]));
    expect(md).toContain("# Put a cat in a PDF");
    expect(md).toContain("## 1. Print the photo to a PDF");
    expect(md).toContain("> The desktop is in OneDrive.");
    expect(md).toContain("```powershell\nStart-Process cat.jpg -Verb Print\n```");
    expect(md).toContain("![Step 1: Print the photo to a PDF](01-make-pdf.png)");
    expect(md).toContain("**This step failed.**");
    expect(md).toContain("Taken out as secrets: GitHub token ×1");
    // A command holding a fence cannot close its own block.
    expect(md).toContain("````powershell\nWrite-Output '```'\n````");
  });

  it("writes the guide as one HTML page, escaped", () => {
    const html = guideHtml(exported, new Map([["make-pdf", "data:image/png;base64,AAAA"]]));
    expect(html).toContain("<title>Put a cat in a PDF</title>");
    expect(html).toContain("<h2>2. Open it &lt;now&gt;</h2>");
    expect(html).toContain(
      '<img src="data:image/png;base64,AAAA" alt="Step 1: Print the photo to a PDF">',
    );
    expect(html).not.toContain("<now>");
    expect(escapeHtml(`<a href="x">'&'</a>`)).toBe(
      "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;",
    );
  });
});
