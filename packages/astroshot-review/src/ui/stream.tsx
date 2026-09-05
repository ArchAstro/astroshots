/**
 * The Shots stream: filter bar, contiguous worktree groups, and one row per
 * image with a live thumbnail.
 */
import { Box, Text } from "ink";

import { durationLabel } from "../data/manifest.js";
import type { Shot } from "../data/model.js";
import { EmptyState, ExecutionPill, MovieBadge, ReviewBadge, WorktreeChip } from "./chrome.js";
import { useServices } from "./context.js";
import { Picture } from "./picture.js";
import type { StreamCounts, StreamFilter, StreamGroup } from "./selectors.js";
import { reviewStateOf } from "./selectors.js";
import { clockTime, theme, truncate } from "./theme.js";

export type StreamItem =
  | { kind: "header"; group: StreamGroup; height: 1 }
  | { kind: "shot"; shot: Shot; group: StreamGroup; height: number };

export const SHOT_ROW_HEIGHT = 4;

export function flattenStream(groups: StreamGroup[], collapsed: Set<string>): StreamItem[] {
  const items: StreamItem[] = [];
  for (const group of groups) {
    items.push({ kind: "header", group, height: 1 });
    if (collapsed.has(group.id)) continue;
    for (const shot of group.shots) items.push({ kind: "shot", shot, group, height: SHOT_ROW_HEIGHT });
  }
  return items;
}

/** Headers of expanded groups are labels, not stops; collapsed ones stay reachable. */
export function isNavigable(item: StreamItem, collapsed: Set<string>): boolean {
  return item.kind === "shot" || collapsed.has(item.group.id);
}

/** Nearest navigable index at or after `from` (direction +1) or before it (−1). */
export function nextNavigable(items: StreamItem[], collapsed: Set<string>, from: number, direction: 1 | -1): number {
  let index = from;
  while (index >= 0 && index < items.length) {
    if (isNavigable(items[index]!, collapsed)) return index;
    index += direction;
  }
  // Fall back to the nearest navigable item in the other direction.
  index = from - direction;
  while (index >= 0 && index < items.length) {
    if (isNavigable(items[index]!, collapsed)) return index;
    index -= direction;
  }
  return Math.max(0, Math.min(from, items.length - 1));
}

/** First item index so the cursor item is fully visible within `height` lines. */
export function scrollWindow(items: StreamItem[], cursor: number, scrollTop: number, height: number): number {
  if (items.length === 0) return 0;
  const clampedCursor = Math.max(0, Math.min(cursor, items.length - 1));
  let top = Math.max(0, Math.min(scrollTop, items.length - 1));
  if (clampedCursor < top) top = clampedCursor;
  const fits = (start: number): boolean => {
    let used = 0;
    for (let index = start; index <= clampedCursor; index += 1) {
      used += items[index]!.height;
      if (used > height) return false;
    }
    return true;
  };
  while (!fits(top) && top < clampedCursor) top += 1;
  return top;
}

export interface StreamProps {
  items: StreamItem[];
  collapsed: Set<string>;
  cursor: number;
  scrollTop: number;
  width: number;
  height: number;
  filter: StreamFilter;
  moviesOnly: boolean;
  counts: StreamCounts;
  focused: boolean;
  scanning: boolean;
  hasRoots: boolean;
  totalShots: number;
  bulkBusy: boolean;
}

function thumbnailCols(cellWidth: number, cellHeight: number): number {
  const rows = SHOT_ROW_HEIGHT - 1;
  const heightPx = rows * cellHeight;
  return Math.max(6, Math.min(16, Math.round((heightPx * (88 / 56)) / cellWidth)));
}

export function FilterBar({ filter, moviesOnly, counts, width, bulkBusy }: Pick<StreamProps, "filter" | "moviesOnly" | "counts" | "width" | "bulkBusy">) {
  const title = filter === "unseen" ? `Unseen (${counts.pending})` : `History (${counts.seen})`;
  return (
    <Box flexShrink={0} width={width} height={1} paddingX={1} justifyContent="space-between">
      <Text bold>{title}</Text>
      <Text>
        {counts.movies > 0 ? (
          <Text color={moviesOnly ? theme.purple : theme.muted} inverse={moviesOnly}>
            {" m Movies "}
          </Text>
        ) : null}
        {filter === "unseen" && counts.pending > 0 ? (
          <Text color={bulkBusy ? theme.muted : theme.green}>{bulkBusy ? " S Marking… " : " S Seen all "}</Text>
        ) : null}
        <Text color={theme.blue}>{filter === "unseen" ? " u History " : " u Unseen "}</Text>
      </Text>
    </Box>
  );
}

function GroupHeader({ group, collapsed, selected, width }: { group: StreamGroup; collapsed: boolean; selected: boolean; width: number }) {
  const unseen = group.shots.filter((shot) => reviewStateOf(shot) !== "seen").length;
  return (
    <Box flexShrink={0} width={width} height={1} paddingLeft={1} backgroundColor={selected ? theme.selection : undefined}>
      <Text color={selected ? theme.brand : theme.muted}>{collapsed ? "▸ " : "▾ "}</Text>
      <WorktreeChip label={group.worktreeShort} />
      <Text color={theme.muted} wrap="truncate">
        {" "}
        {group.worktree !== group.worktreeShort ? `${truncate(group.worktree, Math.max(8, width - 26))} · ` : ""}
        {group.shots.length} {group.shots.length === 1 ? "frame" : "frames"}
        {unseen > 0 ? <Text color={theme.amber}> · {unseen} unseen</Text> : null}
      </Text>
    </Box>
  );
}

function ShotRow({ shot, selected, width, thumbCols, inGroup }: { shot: Shot; selected: boolean; width: number; thumbCols: number; inGroup: boolean }) {
  const state = reviewStateOf(shot);
  const textWidth = Math.max(12, width - thumbCols - 4);
  const time = clockTime(shot.capturedAt);
  const titleWidth = Math.max(8, textWidth - time.length - 2);
  const rowHeight = SHOT_ROW_HEIGHT - 1;
  return (
    <Box flexShrink={0} width={width} height={SHOT_ROW_HEIGHT} flexDirection="column">
      <Box flexShrink={0} width={width} height={rowHeight} flexDirection="row" backgroundColor={selected ? theme.selection : undefined}>
        <Box flexShrink={0} width={1}>
          <Text color={theme.brand}>{selected ? "▎" : " "}</Text>
        </Box>
        <Picture src={shot.path} version={shot.mtimeMs} width={thumbCols} height={rowHeight} label={shot.isMovie ? "▶ movie" : "still"} />
        <Box flexDirection="column" width={textWidth} marginLeft={1} height={rowHeight} overflow="hidden" flexShrink={0}>
          <Box flexShrink={0} width={textWidth} height={1} justifyContent="space-between">
            <Text wrap="truncate">
              {inGroup ? null : <WorktreeChip label={shot.worktreeShort} />}
              {shot.status === "fail" ? <Text color={theme.red}>● </Text> : null}
              <Text bold>{truncate(`${shot.feature} · ${shot.title}`, titleWidth)}</Text>
            </Text>
            <Text color={theme.muted}>{time}</Text>
          </Box>
          <Text color={theme.muted} wrap="truncate">
            {truncate(shot.description || shot.fileName, textWidth)}
          </Text>
          <Text wrap="truncate">
            {shot.isMovie ? (
              <>
                <MovieBadge duration={durationLabel(shot.durationMs)} />
                <Text> </Text>
              </>
            ) : null}
            <ReviewBadge state={state} stale={shot.review?.isStale} />
            {shot.status ? (
              <>
                <Text> </Text>
                <ExecutionPill status={shot.status} />
              </>
            ) : null}
          </Text>
        </Box>
      </Box>
    </Box>
  );
}

export function StreamList(props: StreamProps) {
  const { capabilities } = useServices();
  const { items, collapsed, cursor, scrollTop, width, height, filter, hasRoots, scanning, totalShots } = props;
  const thumbCols = thumbnailCols(capabilities.cellWidth, capabilities.cellHeight);

  if (!hasRoots) {
    return (
      <EmptyState
        width={width}
        title="Choose folders to watch"
        body="Pick the directories that contain your projects. Astroshots watches them for .astroshot/ screenshots."
        action="astroshot review --root <dir>"
      />
    );
  }
  if (totalShots === 0) {
    return scanning ? (
      <EmptyState width={width} title="Scanning watched folders…" body="First scan of a large folder can take a bit. Results stream in as they are found." />
    ) : (
      <EmptyState
        width={width}
        title="Waiting for frames"
        body="When any project under a watched folder writes to .astroshot/, shots land here."
        action="r Rescan"
      />
    );
  }
  if (items.length === 0) {
    return filter === "unseen" ? (
      <EmptyState width={width} title="You’re all caught up" body="Every current frame has been seen." action="u View history" />
    ) : (
      <EmptyState width={width} title="No history yet" body="Frames you mark Seen will appear here." action="u Back to unseen" />
    );
  }

  const rows = [];
  let used = 0;
  for (let index = scrollTop; index < items.length; index += 1) {
    const item = items[index]!;
    if (used + item.height > height) break;
    used += item.height;
    const selected = props.focused && index === cursor;
    if (item.kind === "header") {
      rows.push(<GroupHeader key={`h:${item.group.id}`} group={item.group} collapsed={collapsed.has(item.group.id)} selected={selected} width={width} />);
    } else {
      rows.push(<ShotRow key={item.shot.path} shot={item.shot} selected={selected} width={width} thumbCols={thumbCols} inGroup />);
    }
  }
  const remaining = items.length - (scrollTop + rows.length);
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} overflow="hidden">
      {rows}
      {remaining > 0 && used < height ? (
        <Text color={theme.faint}>  ↓ {remaining} more</Text>
      ) : null}
    </Box>
  );
}
