import { describe, expect, it } from "vitest";

import { KittyGraphicsTracker, encodePng, overlaysToHtml } from "./kitty-graphics.js";
import { createHeadlessTerminal, terminalPlainText } from "./terminal-html.js";

const PNG_1X1 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

function makeTracker(cols = 40, rows = 10) {
  const terminal = createHeadlessTerminal(cols, rows);
  const replies: string[] = [];
  const tracker = new KittyGraphicsTracker({
    terminal,
    cols,
    rows,
    cellWidth: 9,
    cellHeight: 20,
    reply: (data) => replies.push(data),
  });
  return { terminal, tracker, replies };
}

describe("KittyGraphicsTracker", () => {
  it("answers the capability query and size reports", async () => {
    const { tracker, replies } = makeTracker();
    await tracker.write("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[16t\x1b[14t");
    expect(replies).toEqual(["\x1b_Gi=31;OK\x1b\\", "\x1b[6;20;9t", "\x1b[4;200;360t"]);
    expect(tracker.queries.length).toBe(1);
  });

  it("records chunked transmissions and placements at the cursor, stripping APC from text", async () => {
    const { tracker, terminal } = makeTracker();
    const [first, second] = [PNG_1X1.slice(0, 40), PNG_1X1.slice(40)];
    await tracker.write(`hello\x1b_Ga=t,i=5,f=100,q=2,t=d,m=1;${first}\x1b\\`);
    await tracker.write(`\x1b_Gm=0;${second}\x1b\\\x1b7\x1b[3;4H\x1b_Ga=p,i=5,p=1,c=6,r=2,C=1,q=2\x1b\\\x1b8 world`);
    expect(terminalPlainText(terminal, 10)).toBe("hello world");
    const overlays = tracker.overlays();
    expect(overlays.length).toBe(1);
    expect(overlays[0]).toMatchObject({ col: 3, row: 2, cols: 6, rows: 2 });
    expect(overlays[0]!.dataUrl.startsWith("data:image/png;base64,iVBORw0KGgo")).toBe(true);
    expect(overlaysToHtml(overlays, 1.32)).toContain('left:3ch;top:2.64em;width:6ch;height:2.64em');
  });

  it("replaces placements with the same ids and honors deletes", async () => {
    const { tracker } = makeTracker();
    await tracker.write(`\x1b_Ga=t,i=5,f=100,q=2,t=d,m=0;${PNG_1X1}\x1b\\`);
    await tracker.write("\x1b[1;1H\x1b_Ga=p,i=5,p=1,c=2,r=1,q=2\x1b\\\x1b[2;1H\x1b_Ga=p,i=5,p=1,c=3,r=1,q=2\x1b\\");
    expect(tracker.overlays()).toHaveLength(1);
    expect(tracker.overlays()[0]).toMatchObject({ row: 1, cols: 3 });
    await tracker.write("\x1b_Ga=d,d=i,i=5,p=1,q=2\x1b\\");
    expect(tracker.overlays()).toHaveLength(0);
    expect(tracker.imageCount).toBe(1);
    await tracker.write("\x1b_Ga=d,d=A,q=2\x1b\\");
    expect(tracker.imageCount).toBe(0);
  });

  it("buffers a sequence split across writes", async () => {
    const { tracker, terminal } = makeTracker();
    const command = `\x1b_Ga=T,i=9,f=100,q=2,t=d,m=0;${PNG_1X1}\x1b\\`;
    await tracker.write(`a${command.slice(0, 20)}`);
    await tracker.write(`${command.slice(20)}b`);
    expect(terminalPlainText(terminal, 10)).toBe("ab");
    expect(tracker.overlays()).toHaveLength(0); // a=T without c/r is ignored for overlays
    expect(tracker.imageCount).toBe(1);
  });

  it("encodes raw RGB payloads as PNG overlays", async () => {
    const { tracker } = makeTracker();
    const pixels = Buffer.from([255, 0, 0, 0, 255, 0]);
    await tracker.write(`\x1b_Ga=t,i=2,f=24,s=2,v=1,q=2,t=d,m=0;${pixels.toString("base64")}\x1b\\\x1b_Ga=p,i=2,p=1,c=2,r=1,q=2\x1b\\`);
    const overlay = tracker.overlays()[0]!;
    const png = Buffer.from(overlay.dataUrl.split(",")[1]!, "base64");
    expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    expect(png.readUInt32BE(16)).toBe(2);
    expect(encodePng(pixels, 2, 1, 3).equals(png)).toBe(true);
  });
});
