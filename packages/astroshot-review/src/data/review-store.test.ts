import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  addComment,
  markSeen,
  parseReviewDocument,
  readReviewDocument,
  scopedEntry,
  serializeReviewDocument,
  sha256Bytes,
  snapshotFromEntry,
  UnsupportedReviewVersion,
} from "./review-store.js";

let directory: string;

beforeEach(() => {
  directory = fs.mkdtempSync(path.join(os.tmpdir(), "review-store-"));
});

afterEach(() => {
  fs.rmSync(directory, { recursive: true, force: true });
});

describe("review snapshot truth table", () => {
  const sha = "a".repeat(64);
  it("is seen only when the hash matches", () => {
    const seen = snapshotFromEntry({ decision: "seen", image_sha256: sha, comments: [] }, sha);
    expect(seen.state).toBe("seen");
    expect(seen.isStale).toBe(false);
    const stale = snapshotFromEntry({ decision: "seen", image_sha256: sha, comments: [{ id: "1", body: "hi", created_at: "t" }] }, "b".repeat(64));
    expect(stale.state).toBe("pending");
    expect(stale.isStale).toBe(true);
    expect(stale.comments.map((comment) => comment.body)).toEqual(["hi"]);
  });

  it("accepts legacy approved and rejects changes_requested", () => {
    expect(snapshotFromEntry({ decision: "approved", image_sha256: sha }, sha).state).toBe("seen");
    expect(snapshotFromEntry({ decision: "changes_requested", image_sha256: sha }, sha).state).toBe("pending");
  });

  it("treats comment-only entries as pending and not stale", () => {
    const snapshot = snapshotFromEntry({ comments: [{ id: "1", body: "x", created_at: "t" }] }, null);
    expect(snapshot.state).toBe("pending");
    expect(snapshot.isStale).toBe(false);
    expect(snapshot.comments.length).toBe(1);
  });

  it("gates on run id", () => {
    const document = parseReviewDocument(JSON.stringify({ version: 1, run_id: "run-1", reviews: { "a.png": { decision: "seen", image_sha256: sha } } }))!;
    expect(scopedEntry(document, "a.png", "run-1")).not.toBeNull();
    expect(scopedEntry(document, "a.png", "run-2")).toBeNull();
    expect(scopedEntry(document, "a.png", null)).not.toBeNull();
  });

  it("rejects unsupported versions and malformed JSON differently", () => {
    expect(() => parseReviewDocument(JSON.stringify({ version: 2, reviews: {} }))).toThrow(UnsupportedReviewVersion);
    expect(parseReviewDocument("{ nope")).toBeNull();
  });
});

describe("review writes", () => {
  it("marks seen with the app's exact document shape", async () => {
    const image = path.join(directory, "0001-a.png");
    fs.writeFileSync(image, Buffer.from("png-bytes"));
    const now = new Date("2026-09-05T17:42:00.123Z");
    await markSeen({ directory, fileName: "0001-a.png", runId: "run-7", targetPath: image }, { comment: "  Looks clipped  ", now });
    const raw = fs.readFileSync(path.join(directory, "review.json"), "utf8");
    const parsed = JSON.parse(raw);
    expect(Object.keys(parsed)).toEqual(["reviews", "run_id", "updated_at", "version"]);
    expect(parsed.version).toBe(1);
    expect(parsed.run_id).toBe("run-7");
    expect(parsed.updated_at).toBe("2026-09-05T17:42:00Z");
    const entry = parsed.reviews["0001-a.png"];
    expect(entry.decision).toBe("seen");
    expect(entry.reviewed_at).toBe("2026-09-05T17:42:00Z");
    expect(entry.image_sha256).toBe(sha256Bytes(Buffer.from("png-bytes")));
    expect(entry.comments.length).toBe(1);
    expect(entry.comments[0].body).toBe("Looks clipped");
    expect(entry.comments[0].id).toMatch(/^[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$/);
    expect(raw.endsWith("\n")).toBe(true);
    expect(fs.readdirSync(directory).filter((name) => name.startsWith(".review.tmp"))).toEqual([]);
  });

  it("adds comments without inventing a decision and appends in order", async () => {
    const image = path.join(directory, "0002-b.png");
    fs.writeFileSync(image, "x");
    await addComment({ directory, fileName: "0002-b.png", runId: null, targetPath: image }, "first");
    await addComment({ directory, fileName: "0002-b.png", runId: null, targetPath: image }, "second");
    const document = (await readReviewDocument(directory))!;
    const entry = document.reviews["0002-b.png"]!;
    expect(entry.decision).toBeUndefined();
    expect(entry.image_sha256).toBeUndefined();
    expect(entry.comments!.map((comment) => comment.body)).toEqual(["first", "second"]);
  });

  it("wipes prior reviews when the run id changes", async () => {
    const image = path.join(directory, "0001-a.png");
    fs.writeFileSync(image, "x");
    await markSeen({ directory, fileName: "0001-a.png", runId: "run-1", targetPath: image });
    await addComment({ directory, fileName: "0001-a.png", runId: "run-2", targetPath: image }, "new run");
    const document = (await readReviewDocument(directory))!;
    expect(document.run_id).toBe("run-2");
    expect(document.reviews["0001-a.png"]!.decision).toBeUndefined();
    expect(document.reviews["0001-a.png"]!.comments!.length).toBe(1);
  });

  it("refuses to overwrite a review.json it cannot parse", async () => {
    fs.writeFileSync(path.join(directory, "review.json"), "{ broken");
    const image = path.join(directory, "0001-a.png");
    fs.writeFileSync(image, "x");
    await expect(markSeen({ directory, fileName: "0001-a.png", runId: null, targetPath: image })).rejects.toThrow(/not valid JSON/);
  });

  it("serializes with sorted keys and two-space indentation", () => {
    const text = serializeReviewDocument({ version: 1, run_id: "r", reviews: { "b.png": { decision: "seen" }, "a.png": {} } });
    expect(text.indexOf('"a.png"')).toBeLessThan(text.indexOf('"b.png"'));
    expect(text.startsWith("{\n  \"reviews\"")).toBe(true);
  });
});
