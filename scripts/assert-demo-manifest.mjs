#!/usr/bin/env node
/**
 * Validate an `astroshot demo` feature directory against the documented
 * .astroshot manifest contract (skills/astroshots-review/references/manifest.md).
 */
import fs from "node:fs";
import path from "node:path";

const featureDirectory = process.argv[2];
if (!featureDirectory) {
  throw new Error("usage: assert-demo-manifest.mjs <.astroshot/feature-dir>");
}

const manifest = JSON.parse(
  fs.readFileSync(path.join(featureDirectory, "manifest.json"), "utf8"),
);

if (manifest.version !== 1) throw new Error("manifest.version must be 1");
if (manifest.feature !== path.basename(featureDirectory)) {
  throw new Error("manifest.feature must match the directory name");
}
if (typeof manifest.run_id !== "string" || !manifest.run_id) {
  throw new Error("manifest.run_id must be a non-empty string");
}
if (!["running", "pass", "fail", "idle"].includes(manifest.status)) {
  throw new Error(`unexpected manifest.status: ${manifest.status}`);
}
if (!Array.isArray(manifest.shots) || manifest.shots.length < 2) {
  throw new Error("demo must write more than one shot");
}

const stills = [];
const movies = [];
for (const shot of manifest.shots) {
  if (shot.file !== path.basename(shot.file)) {
    throw new Error(`shot.file must be a basename: ${shot.file}`);
  }
  if (!/^\d{4}-[a-z0-9-]+\.png$/.test(shot.file)) {
    throw new Error(`shot.file must be NNNN-slug.png: ${shot.file}`);
  }
  if (shot.id !== shot.file.slice(0, 4)) {
    throw new Error(`shot.id must match the sequence prefix: ${shot.file}`);
  }
  const image = path.join(featureDirectory, shot.file);
  const bytes = fs.readFileSync(image);
  if (bytes.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a") {
    throw new Error(`shot file is not a PNG: ${shot.file}`);
  }
  if (Number.isNaN(Date.parse(shot.captured_at))) {
    throw new Error(`shot.captured_at is not ISO-8601: ${shot.file}`);
  }
  (shot.video ? movies : stills).push(shot);
}

if (stills.length < 1) throw new Error("demo must write at least one still");
if (movies.length !== 1) throw new Error("demo must write exactly one movie");

const [movie] = movies;
if (movie.kind !== "movie") throw new Error("movie shot needs kind: movie");
if (movie.video !== path.basename(movie.video)) {
  throw new Error("movie.video must be a basename");
}
if (!/\.(webm|mp4|mov)$/.test(movie.video)) {
  throw new Error(`unsupported movie container: ${movie.video}`);
}
if (path.parse(movie.video).name !== path.parse(movie.file).name) {
  throw new Error("movie video must be the poster's sibling");
}
const video = path.join(featureDirectory, movie.video);
if (!fs.statSync(video).size) throw new Error(`empty movie video: ${video}`);
if (!(movie.duration_ms > 0)) throw new Error("movie needs a positive duration_ms");
if (!["browser", "pty", "desktop.window", "frames"].includes(movie.source)) {
  throw new Error(`unknown movie source: ${movie.source}`);
}
for (const chapter of movie.chapters ?? []) {
  if (!/^[a-z0-9-]+$/.test(chapter.slug ?? "")) {
    throw new Error(`chapter.slug must be kebab-case: ${chapter.slug}`);
  }
  if (typeof chapter.t_ms !== "number") {
    throw new Error(`chapter.t_ms must be a number: ${chapter.slug}`);
  }
}

const leftovers = fs
  .readdirSync(featureDirectory)
  .filter((entry) => entry.startsWith("."));
if (leftovers.length) {
  throw new Error(`atomic-write leftovers remain: ${leftovers.join(", ")}`);
}
