/**
 * Friction Logs: scenario list, scenario detail with run picker and steps,
 * step detail, and the step takeover.
 */
import path from "node:path";

import { Box, Text } from "ink";

import { frictionStatusLabel, runDisplayTitle, stepCountLabel } from "../data/friction.js";
import type { FrictionLog, FrictionRun, FrictionStep } from "../data/model.js";
import { EmptyState, MetaRow, Rule, SectionLabel, StatusPill, WorktreeChip } from "./chrome.js";
import { Picture } from "./picture.js";
import type { StreamFilter } from "./selectors.js";
import { frictionState, frictionSummary, latestRun } from "./selectors.js";
import { relativeTime, theme, truncate } from "./theme.js";

export const FRICTION_ROW_HEIGHT = 4;

export interface FrictionListProps {
  logs: FrictionLog[];
  cursor: number;
  scrollTop: number;
  width: number;
  height: number;
  filter: StreamFilter;
  counts: { pending: number; seen: number };
  focused: boolean;
  total: number;
  now: number;
}

export function FrictionFilterBar({ filter, counts, width }: Pick<FrictionListProps, "filter" | "counts" | "width">) {
  const title = filter === "unseen" ? `Unseen (${counts.pending})` : `History (${counts.seen})`;
  return (
    <Box flexShrink={0} width={width} height={1} paddingX={1} justifyContent="space-between">
      <Text bold>{title}</Text>
      <Text>
        {filter === "unseen" && counts.pending > 0 ? <Text color={theme.green}> S Seen all </Text> : null}
        <Text color={theme.blue}>{filter === "unseen" ? " u History " : " u Unseen "}</Text>
      </Text>
    </Box>
  );
}

function FrictionRow({ log, selected, width, now }: { log: FrictionLog; selected: boolean; width: number; now: number }) {
  const run = latestRun(log);
  const inner = width - 3;
  const status = frictionStatusLabel(log.status);
  const footer = run
    ? log.runs.length > 1
      ? `${log.runs.length} runs · ${runDisplayTitle(run.runId)}`
      : runDisplayTitle(run.runId)
    : "Prompt only · no runs yet";
  return (
    <Box flexShrink={0} width={width} height={FRICTION_ROW_HEIGHT} flexDirection="row" backgroundColor={selected ? theme.selection : undefined}>
      <Box width={1} flexShrink={0}>
        <Text color={theme.brand}>{selected ? "▎" : " "}</Text>
      </Box>
      <Box flexDirection="column" width={inner} marginLeft={1} overflow="hidden">
        <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
          <Text bold wrap="truncate">
            {truncate(log.title, Math.max(8, inner - 12))}
          </Text>
          <StatusPill label={status} />
        </Box>
        <Text color={theme.muted} wrap="truncate">
          {truncate(log.description, inner)}
        </Text>
        <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
          <Text wrap="truncate">
            <WorktreeChip label={log.worktreeShort} />
            <Text color={theme.muted}> {log.slug}</Text>
          </Text>
          <Text color={theme.muted}>{frictionSummary(log)}</Text>
        </Box>
        <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
          <Text color={theme.muted} wrap="truncate">
            {footer}
          </Text>
          <Text color={frictionState(log) === "seen" ? theme.blue : theme.amber}>
            {frictionState(log) === "seen" ? "Seen" : "Unseen"}
            {run ? <Text color={theme.muted}> · {relativeTime(run.capturedAt, now)}</Text> : null}
          </Text>
        </Box>
      </Box>
    </Box>
  );
}

export function frictionScroll(count: number, cursor: number, scrollTop: number, height: number): number {
  const perPage = Math.max(1, Math.floor(height / FRICTION_ROW_HEIGHT));
  let top = Math.min(scrollTop, Math.max(0, count - 1));
  if (cursor < top) top = cursor;
  if (cursor >= top + perPage) top = cursor - perPage + 1;
  return Math.max(0, top);
}

export function FrictionList(props: FrictionListProps) {
  const { logs, cursor, scrollTop, width, height, filter, focused, total, now } = props;
  if (total === 0) {
    return (
      <Box flexShrink={0} flexDirection="column" width={width} height={height}>
        <EmptyState
          width={width}
          title="No friction logs yet"
          body="Author a scenario with the friction-log skill, then run it. Results land under .astroshot/friction-logs/."
        />
        <Box flexDirection="column" paddingX={2}>
          <Text color={theme.muted}>.astroshot/friction-logs/{"<slug>"}/prompt.md</Text>
          <Text color={theme.muted}>runs/{"<run-id>"}/log.jsonl + screenshots</Text>
        </Box>
      </Box>
    );
  }
  if (logs.length === 0) {
    return filter === "unseen" ? (
      <EmptyState width={width} title="You’re all caught up" body="Every friction log has been seen." action="u View history" />
    ) : (
      <EmptyState width={width} title="No history yet" body="Logs you mark Seen will appear here." action="u Back to unseen" />
    );
  }
  const perPage = Math.max(1, Math.floor(height / FRICTION_ROW_HEIGHT));
  const visible = logs.slice(scrollTop, scrollTop + perPage);
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} overflow="hidden">
      {visible.map((log, offset) => (
        <FrictionRow key={log.id} log={log} selected={focused && scrollTop + offset === cursor} width={width} now={now} />
      ))}
    </Box>
  );
}

export interface FrictionLogDetailProps {
  log: FrictionLog;
  run: FrictionRun | null;
  stepCursor: number;
  promptOpen: boolean;
  prompt: string | null;
  width: number;
  height: number;
}

export function FrictionLogDetail({ log, run, stepCursor, promptOpen, prompt, width, height }: FrictionLogDetailProps) {
  const inner = width - 2;
  const runIndex = run ? log.runs.indexOf(run) : -1;
  const improve = run ? run.steps.flatMap((step) => step.improve.map((note) => ({ step: step.step, note }))) : [];
  const headerLines = 6 + (promptOpen ? 6 : 0) + (improve.length > 0 ? Math.min(improve.length, 3) + 1 : 0);
  const stepsHeight = Math.max(3, height - headerLines - 2);
  const perPage = Math.max(1, Math.floor(stepsHeight / 2));
  const stepsTop = Math.max(0, Math.min(stepCursor - perPage + 1, run ? run.steps.length - perPage : 0));
  const steps = run ? run.steps.slice(Math.max(0, Math.min(stepsTop, stepCursor)), Math.max(0, Math.min(stepsTop, stepCursor)) + perPage) : [];
  const firstIndex = Math.max(0, Math.min(stepsTop, stepCursor));

  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} paddingX={1} overflow="hidden">
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <Text color={theme.blue}>‹ Logs</Text>
        <Text color={theme.muted}>
          {run ? stepCountLabel(run.steps.length) : "no runs"}
          {frictionState(log) !== "seen" && run ? <Text color={theme.green}>   s Seen</Text> : null}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <Text bold wrap="truncate">
          {truncate(log.title, inner - 12)}
        </Text>
        <StatusPill label={frictionStatusLabel(log.status)} />
      </Box>
      <Text color={theme.muted} wrap="truncate">
        {truncate(log.description, inner)}
      </Text>
      <Text wrap="truncate">
        <WorktreeChip label={log.worktreeShort} />
        <Text color={theme.muted}> {log.slug}</Text>
        {log.promptPath ? (
          <Text color={theme.blue}>
            {"   p "}
            {promptOpen ? "Hide prompt" : "Prompt"}
          </Text>
        ) : null}
      </Text>
      {promptOpen && prompt !== null ? (
        <Box flexShrink={0} flexDirection="column" width={inner} height={6} overflow="hidden" borderStyle="round" borderColor={theme.faint} paddingX={1}>
          <SectionLabel>SCENARIO PROMPT</SectionLabel>
          <Text color={theme.muted} wrap="wrap">
            {truncate(prompt, (inner - 4) * 4)}
          </Text>
        </Box>
      ) : null}
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <Text>
          <Text bold>{log.runs.length > 1 ? `Runs ${log.runs.length}` : "Run"}</Text>
          {run ? (
            <Text color={theme.muted}>
              {"  "}
              {runDisplayTitle(run.runId)}
              {runIndex === 0 ? <Text color={theme.purple}> Latest</Text> : null}
              {"  "}
              {run.runId}
            </Text>
          ) : null}
        </Text>
        {log.runs.length > 1 ? <Text color={theme.blue}>[ ] switch run</Text> : null}
      </Box>
      {improve.length > 0 ? (
        <Box flexDirection="column" width={inner}>
          <Text color={theme.amber} bold>
            Improve rollup · {improve.length}
          </Text>
          {improve.slice(0, 3).map((item, index) => (
            <Text key={`${item.step}-${index}`} color={theme.amber} wrap="truncate">
              {String(item.step).padStart(2, "0")} {truncate(item.note, inner - 3)}
            </Text>
          ))}
        </Box>
      ) : null}
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <SectionLabel>STEPS</SectionLabel>
        <Text color={theme.muted}>⏎ open step · ↑↓ move</Text>
      </Box>
      {!run ? (
        <Box flexDirection="column" width={inner}>
          <Text bold>No runs yet</Text>
          <Text color={theme.muted} wrap="wrap">
            This scenario has a prompt but no log.jsonl run. Use the friction-log skill to execute it; steps will appear here as the agent writes them.
          </Text>
        </Box>
      ) : run.steps.length === 0 ? (
        <Text color={theme.muted}>No JSONL steps in this run</Text>
      ) : (
        <Box flexDirection="column" width={inner} overflow="hidden" flexGrow={1}>
          {steps.map((step, offset) => (
            <StepRow key={step.id} step={step} selected={firstIndex + offset === stepCursor} width={inner} />
          ))}
        </Box>
      )}
    </Box>
  );
}

function StepRow({ step, selected, width }: { step: FrictionStep; selected: boolean; width: number }) {
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={2} backgroundColor={selected ? theme.selection : undefined}>
      <Box flexShrink={0} height={1} justifyContent="space-between" width={width}>
        <Text wrap="truncate">
          <Text color={selected ? theme.brand : theme.purple}>{String(step.step).padStart(2, "0")}</Text>
          {"  "}
          <Text bold>{truncate(step.title, width - 16)}</Text>
        </Text>
        <Text>
          {step.transcript ? <Text color={theme.purple}>✎ </Text> : null}
          {step.screenshots.length > 0 ? <Text color={theme.muted}>◫ </Text> : null}
          {step.good.length > 0 ? <Text color={theme.green}>{step.good.length}+ </Text> : null}
          {step.improve.length > 0 ? <Text color={theme.amber}>{step.improve.length}! </Text> : null}
        </Text>
      </Box>
      <Text color={theme.muted} wrap="truncate">
        {"    "}
        {truncate(step.description, width - 4)}
      </Text>
    </Box>
  );
}

export interface FrictionStepDetailProps {
  log: FrictionLog;
  run: FrictionRun;
  stepIndex: number;
  imageIndex: number;
  width: number;
  height: number;
  mode: "page" | "takeover";
}

function NoteCard({ title, color, items, empty, width }: { title: string; color: string; items: string[]; empty: string; width: number }) {
  return (
    <Box flexDirection="column" width={width}>
      <Text color={color} bold>
        {title}
        {items.length > 0 ? <Text color={theme.muted}> {items.length}</Text> : null}
      </Text>
      {items.length === 0 ? (
        <Text color={theme.muted}>{empty}</Text>
      ) : (
        items.slice(0, 4).map((item, index) => (
          <Text key={index} wrap="truncate">
            {"• "}
            {truncate(item, width - 2)}
          </Text>
        ))
      )}
    </Box>
  );
}

export function FrictionStepDetail({ log, run, stepIndex, imageIndex, width, height }: FrictionStepDetailProps) {
  const step = run.steps[stepIndex]!;
  const inner = width - 2;
  const preview = Math.max(6, Math.min(20, Math.round(height * 0.38)));
  const screenshot = step.screenshots[Math.min(imageIndex, step.screenshots.length - 1)] ?? null;
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} paddingX={1} overflow="hidden">
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <Text color={theme.blue}>‹ Steps</Text>
        <Text color={theme.muted}>
          {stepIndex + 1} / {run.steps.length}
          {"   ← → step"}
        </Text>
      </Box>
      <Box flexShrink={0} width={inner} height={preview}>
        <Picture src={screenshot} width={inner} height={preview} maxUpscale={8} label="No screenshot for this step" />
      </Box>
      {step.screenshots.length > 1 ? (
        <Text color={theme.muted}>
          {step.screenshots.map((_, index) => (index === imageIndex ? "● " : "○ "))}
          {imageIndex + 1} of {step.screenshots.length} · [ ] image
        </Text>
      ) : null}
      <Text wrap="truncate">
        <Text color={theme.purple} bold>
          {String(step.step).padStart(2, "0")}
        </Text>
        {"  "}
        <Text bold>{truncate(step.title, inner - 4)}</Text>
      </Text>
      {step.description ? (
        <Text color={theme.muted} wrap="truncate">
          {truncate(step.description, inner)}
        </Text>
      ) : null}
      {step.url ? <Text color={theme.blue}>{step.url}</Text> : null}
      <Box flexDirection="column" width={inner} overflow="hidden" flexGrow={1} flexShrink={1} minHeight={0}>
        {step.transcript ? (
          <Box flexDirection="column" width={inner}>
            <Text color={theme.purple} bold>
              Transcript
            </Text>
            <Text wrap="wrap">{truncate(step.transcript, inner * 3)}</Text>
          </Box>
        ) : null}
        <NoteCard title="Looks good" color={theme.green} items={step.good} empty="No positives noted for this step" width={inner} />
        <NoteCard title="Can improve" color={theme.amber} items={step.improve} empty="No friction found for this step" width={inner} />
        <Rule width={inner} />
        <MetaRow label="Log" value={log.slug} width={inner} />
        <MetaRow label="Run" value={run.runId} width={inner} />
        <MetaRow label="Tree" value={log.worktree} width={inner} />
        <MetaRow label="File" value={screenshot ? path.basename(screenshot) : "—"} width={inner} />
      </Box>
    </Box>
  );
}

export function FrictionStepTakeover({ log, run, stepIndex, imageIndex, width, height }: FrictionStepDetailProps) {
  const step = run.steps[stepIndex]!;
  const railWidth = width >= 100 ? 42 : Math.max(28, Math.floor(width * 0.38));
  const stageWidth = Math.max(10, width - railWidth - 1);
  const stageHeight = Math.max(4, height - 3);
  const screenshot = step.screenshots[Math.min(imageIndex, step.screenshots.length - 1)] ?? null;
  const meta = [log.title, log.worktreeShort, run.runId, step.url ?? ""].filter(Boolean).join(" · ");
  const railInner = railWidth - 2;
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height}>
      <Box flexShrink={0} height={1} paddingX={1} justifyContent="space-between" width={width}>
        <Text wrap="truncate">
          <Text backgroundColor={theme.selection} color={theme.purple}>
            {" "}
            {String(step.step).padStart(2, "0")}{" "}
          </Text>
          {"  "}
          <Text bold>{truncate(step.title, width - 40)}</Text>
        </Text>
        <Text color={theme.muted}>
          <Text color={theme.blue}>‹</Text> {stepIndex + 1} / {run.steps.length} <Text color={theme.blue}>›</Text>
          {"   esc close"}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} paddingX={1} width={width}>
        <Text color={theme.muted} wrap="truncate">
          {truncate(meta, width - 2)}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} paddingX={1} width={width}>
        <Rule width={width - 2} />
      </Box>
      <Box flexShrink={0} flexDirection="row" width={width} height={stageHeight}>
        <Box flexShrink={0} flexDirection="column" width={stageWidth} height={stageHeight} alignItems="center" justifyContent="center">
          <Picture src={screenshot} width={stageWidth} height={step.screenshots.length > 1 ? stageHeight - 1 : stageHeight} maxUpscale={8} label="No screenshot for this step" />
          {step.screenshots.length > 1 ? (
            <Text color={theme.muted}>
              Image {imageIndex + 1} / {step.screenshots.length} · [ ] switch
            </Text>
          ) : null}
        </Box>
        <Box flexShrink={0} width={1} height={stageHeight} flexDirection="column">
          {Array.from({ length: stageHeight }, (_, index) => (
            <Text key={index} color={theme.faint}>
              │
            </Text>
          ))}
        </Box>
        <Box flexShrink={0} flexDirection="column" width={railWidth} height={stageHeight} paddingX={1} overflow="hidden">
          <Text bold>Step notes</Text>
          <Text color={theme.muted} wrap="wrap">
            {truncate(step.description || "—", railInner * 3)}
          </Text>
          {step.url ? <Text color={theme.blue}>{step.url}</Text> : null}
          <Rule width={railInner} />
          <Text color={theme.purple} bold>
            Transcript
          </Text>
          <Text wrap="wrap">{truncate(step.transcript || "—", railInner * 5)}</Text>
          <Rule width={railInner} />
          <NoteCard title="Looks good" color={theme.green} items={step.good} empty="No positives noted" width={railInner} />
          <NoteCard title="Can improve" color={theme.amber} items={step.improve} empty="No friction found" width={railInner} />
        </Box>
      </Box>
    </Box>
  );
}
