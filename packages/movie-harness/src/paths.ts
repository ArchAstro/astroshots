import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

export function resolveRoot(root?: string): string {
  if (root) return path.resolve(root);
  try {
    return execFileSync("git", ["rev-parse", "--show-toplevel"], {
      encoding: "utf8",
    }).trim();
  } catch {
    return process.cwd();
  }
}

export function assertKebabCase(name: string, label: string): void {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
    throw new Error(`${label} must be kebab-case [a-z0-9-]+, got ${JSON.stringify(name)}`);
  }
}

export function assertSlug(slug: string): void {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(slug)) {
    throw new Error(`slug must be kebab-case [a-z0-9-]+, got ${JSON.stringify(slug)}`);
  }
}

export function featureDir(root: string, feature: string): string {
  return path.join(root, ".astroshot", feature);
}

export function movieStateDir(root: string, feature: string): string {
  return path.join(featureDir(root, feature), ".movie");
}

export function defaultRunId(feature: string): string {
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d+Z$/, "Z");
  return `${feature}-${stamp}-${process.pid}`;
}

export function humanize(slug: string): string {
  return slug
    .split(/[-_]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

export function ensureDir(dir: string): void {
  fs.mkdirSync(dir, { recursive: true });
}

export function nextSequence(featureDirectory: string): string {
  const entries = fs.existsSync(featureDirectory)
    ? fs.readdirSync(featureDirectory)
    : [];
  let max = 0;
  for (const name of entries) {
    const match = /^(\d{4})-/.exec(name);
    if (match) max = Math.max(max, Number(match[1]));
  }
  return String(max + 1).padStart(4, "0");
}
