import { Box, Text } from "ink";
import type { ReactNode } from "react";

import type { ReviewState } from "../data/model.js";
import { theme, truncate } from "./theme.js";

export function WorktreeChip({ label }: { label: string }) {
  return (
    <Text backgroundColor={theme.selection} color={theme.purple}>
      {` ${label} `}
    </Text>
  );
}

export function ReviewBadge({ state, stale }: { state: ReviewState; stale?: boolean }) {
  if (state === "seen") return <Text color={theme.blue}>● Seen</Text>;
  return <Text color={theme.amber}>{stale ? "● Unseen · changed" : "● Unseen"}</Text>;
}

export function MovieBadge({ duration }: { duration: string | null }) {
  return (
    <Text backgroundColor="#3b2f6e" color={theme.purple}>
      {duration ? ` Movie · ${duration} ` : " Movie "}
    </Text>
  );
}

export function StatusPill({ label }: { label: string | null }) {
  if (!label) return null;
  const color =
    label === "Complete"
      ? theme.green
      : label === "Running"
        ? theme.amber
        : label === "Failed"
          ? theme.red
          : label === "Ready"
            ? theme.blue
            : theme.muted;
  return <Text color={color}>{label}</Text>;
}

export function ExecutionPill({ status }: { status: string | null }) {
  if (!status) return null;
  const color = status === "pass" ? theme.green : status === "fail" ? theme.red : status === "running" ? theme.amber : theme.muted;
  return <Text color={color}>· run {status}</Text>;
}

export interface KeyHint {
  key: string;
  label: string;
}

export function HintBar({ hints, width }: { hints: KeyHint[]; width: number }) {
  const parts: ReactNode[] = [];
  let used = 0;
  hints.forEach((hint, index) => {
    const length = hint.key.length + 1 + hint.label.length + 3;
    if (used + length > width) return;
    used += length;
    parts.push(
      <Text key={`${hint.key}-${index}`}>
        <Text color={theme.brand} bold>
          {hint.key}
        </Text>
        <Text color={theme.muted}> {hint.label}   </Text>
      </Text>,
    );
  });
  return (
    <Box flexShrink={0} width={width} height={1} overflow="hidden">
      <Text wrap="truncate">{parts}</Text>
    </Box>
  );
}

export function Toast({ message, width }: { message: string | null; width: number }) {
  if (!message) return null;
  const text = ` ${truncate(message, Math.max(8, width - 6))} `;
  return (
    <Box flexShrink={0} width={width} justifyContent="center" height={1}>
      <Text backgroundColor="#2e3140" color={theme.text} bold>
        {text}
      </Text>
    </Box>
  );
}

/** One terminal row that never collapses when its column overflows. */
export function Line({ children, width }: { children: ReactNode; width?: number }) {
  return (
    <Box flexShrink={0} height={1} width={width} overflow="hidden">
      {children}
    </Box>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <Text color={theme.muted} bold>
      {children}
    </Text>
  );
}

export function Rule({ width, color }: { width: number; color?: string }) {
  return <Text color={color ?? theme.faint}>{"─".repeat(Math.max(0, width))}</Text>;
}

export function MetaRow({ label, value, width }: { label: string; value: string; width: number }) {
  const labelWidth = 10;
  return (
    <Box flexShrink={0} width={width} height={1}>
      <Box width={labelWidth} flexShrink={0}>
        <Text color={theme.muted}>{label}</Text>
      </Box>
      <Box flexGrow={1} overflow="hidden">
        <Text wrap="truncate-start">{value}</Text>
      </Box>
    </Box>
  );
}

export function EmptyState({
  title,
  body,
  action,
  width,
}: {
  title: string;
  body: string;
  action?: string;
  width: number;
}) {
  return (
    <Box flexDirection="column" alignItems="center" justifyContent="center" flexGrow={1} paddingX={2} width={width}>
      <Text bold>{title}</Text>
      <Box width={Math.min(width - 4, 60)} justifyContent="center">
        <Text color={theme.muted} wrap="wrap">
          {body}
        </Text>
      </Box>
      {action ? (
        <Box marginTop={1}>
          <Text color={theme.brand}>{action}</Text>
        </Box>
      ) : null}
    </Box>
  );
}
