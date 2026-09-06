import { describe, expect, it } from "vitest";

import { chunkPayload, encodeDelete, encodePlace, encodeQuery, encodeTransmit, parseGraphicsCommand } from "./kitty.js";

describe("kitty graphics encoder", () => {
  it("chunks base64 payloads at the protocol limit with continuation flags", () => {
    const data = Buffer.alloc(5000, 1);
    const output = encodeTransmit({ id: 7, format: 100, data });
    const commands = [...output.matchAll(/\x1b_G([^\x1b]*)\x1b\\/g)].map((match) => parseGraphicsCommand(match[1]!));
    expect(commands.length).toBe(2);
    expect(commands[0]!.keys).toMatchObject({ a: "t", i: "7", f: "100", t: "d", m: "1", q: "2" });
    expect(commands[1]!.keys).toEqual({ m: "0" });
    expect(commands[0]!.payload.length).toBe(4096);
    expect(Buffer.from(commands.map((command) => command.payload).join(""), "base64").equals(data)).toBe(true);
  });

  it("uses the file medium when a path is given", () => {
    const output = encodeTransmit({ id: 3, format: 100, data: Buffer.alloc(0), filePath: "/tmp/a.png" });
    const command = parseGraphicsCommand(output.slice(3, -2));
    expect(command.keys.t).toBe("f");
    expect(Buffer.from(command.payload, "base64").toString()).toBe("/tmp/a.png");
  });

  it("declares raw dimensions and compression for RGB payloads", () => {
    const output = encodeTransmit({ id: 1, format: 24, width: 2, height: 1, compressed: true, data: Buffer.from([0, 0, 0, 0, 0, 0]) });
    expect(output).toContain("f=24");
    expect(output).toContain("s=2,v=1");
    expect(output).toContain("o=z");
  });

  it("places without moving the cursor and quietly", () => {
    expect(encodePlace({ id: 9, placementId: 2, cols: 10, rows: 3 })).toBe("\x1b_Ga=p,i=9,p=2,c=10,r=3,z=0,C=1,q=2\x1b\\");
  });

  it("encodes every delete form", () => {
    expect(encodeDelete({ kind: "placement", id: 9, placementId: 2 })).toBe("\x1b_Ga=d,d=i,i=9,p=2,q=2\x1b\\");
    expect(encodeDelete({ kind: "image", id: 9 })).toBe("\x1b_Ga=d,d=I,i=9,q=2\x1b\\");
    expect(encodeDelete({ kind: "all-placements" })).toBe("\x1b_Ga=d,d=a,q=2\x1b\\");
    expect(encodeDelete({ kind: "all" })).toBe("\x1b_Ga=d,d=A,q=2\x1b\\");
  });

  it("formats the capability query kitty documents", () => {
    expect(encodeQuery(31)).toBe("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
  });

  it("chunks empty payloads as one empty chunk", () => {
    expect(chunkPayload("")).toEqual([""]);
  });
});
