#!/usr/bin/env node
/**
 * Validate an `astroshot doctor --json` report: every documented check is
 * present, each carries a remediation line, and the exit contract is keyed to
 * required checks only.
 */
import fs from "node:fs";

const reportPath = process.argv[2];
if (!reportPath) {
  throw new Error("usage: assert-doctor-report.mjs <doctor.json>");
}

const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
if (typeof report.project !== "string" || !report.project) {
  throw new Error("doctor report must name the project it checked");
}
if (typeof report.ok !== "boolean") throw new Error("doctor report needs ok");
if (!Array.isArray(report.checks)) throw new Error("doctor report needs checks");

const expected = [
  "node",
  "watch-roots",
  "app",
  "app-running",
  "chromium",
  "screen-recording",
];
for (const id of expected) {
  const check = report.checks.find((entry) => entry.id === id);
  if (!check) throw new Error(`doctor is missing the ${id} check`);
  if (!["pass", "fail", "warn", "skip"].includes(check.status)) {
    throw new Error(`${id} has an unknown status: ${check.status}`);
  }
  if (typeof check.required !== "boolean") {
    throw new Error(`${id} must declare whether it is required`);
  }
  if (!check.title || !check.detail || !check.remediation) {
    throw new Error(`${id} must report a title, detail, and remediation`);
  }
}

const requiredFailures = report.checks
  .filter((check) => check.required && check.status === "fail")
  .map((check) => check.id);
if (report.ok !== (requiredFailures.length === 0)) {
  throw new Error("doctor.ok must reflect required failures only");
}
if (
  requiredFailures.sort().join(",") !== [...report.failures].sort().join(",")
) {
  throw new Error("doctor.failures must list exactly the required failures");
}
