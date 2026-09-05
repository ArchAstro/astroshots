/**
 * Shot detail: preview, heading, movie actions, chapters, feedback, Seen,
 * and the metadata card — the tray's Detail pane, in the app's order.
 */
import path from "node:path";

import { Box, Text } from "ink";

import { chapterTimeLabel, durationLabel } from "../data/manifest.js";
import type { Shot } from "../data/model.js";
import { Line, MetaRow, MovieBadge, ReviewBadge, Rule, SectionLabel } from "./chrome.js";
import { useServices } from "./context.js";
import { MoviePlayer, ProgressBar, type PlaybackState } from "./movie-player.js";
import { Picture } from "./picture.js";
import { reviewStateOf } from "./selectors.js";
import { TextInput } from "./text-input.js";
import { abbreviatedDateTime, isoDateTime, theme, truncate } from "./theme.js";

export interface DetailPaneProps {
  shot: Shot;
  position: { index: number; count: number };
  width: number;
  height: number;
  /** `page` shows the back/paging header; `pane` is the split-view column. */
  mode: "pane" | "page";
  composer: boolean;
  onComposerSubmit: (text: string) => void;
  onComposerCancel: () => void;
  inlinePlayer: boolean;
  playback: PlaybackState;
  onPlayback: (patch: Partial<PlaybackState>) => void;
  busy: boolean;
  error: string | null;
  /** 1 fills the preview; below shrinks toward native size. */
  zoom: number;
}

export function previewHeight(height: number): number {
  return Math.max(6, Math.min(22, Math.round(height * 0.4)));
}

export function CommentList({ shot, width, max }: { shot: Shot; width: number; max: number }) {
  const comments = shot.review?.comments ?? [];
  if (comments.length === 0) {
    return (
      <Line width={width}>
        <Text color={theme.muted} wrap="truncate">
          No comments yet · leave concise, actionable feedback for this frame.
        </Text>
      </Line>
    );
  }
  const visible = comments.slice(-max);
  const hidden = comments.length - visible.length;
  return (
    <Box flexDirection="column" width={width} flexShrink={0}>
      {hidden > 0 ? (
        <Line width={width}>
          <Text color={theme.faint}>… {hidden} earlier</Text>
        </Line>
      ) : null}
      {visible.map((comment) => (
        <Box key={comment.id} flexDirection="column" width={width} flexShrink={0}>
          <Line width={width}>
            <Text color={theme.muted} wrap="truncate">
              <Text color={theme.purple}>Reviewer</Text> · {abbreviatedDateTime(comment.createdAt)}
            </Text>
          </Line>
          <Box flexShrink={0} width={width} height={Math.min(3, Math.max(1, Math.ceil(comment.body.length / Math.max(10, width))))} overflow="hidden">
            <Text wrap="wrap">{truncate(comment.body, width * 3)}</Text>
          </Box>
        </Box>
      ))}
    </Box>
  );
}

export function DetailPane(props: DetailPaneProps) {
  const { shot, position, width, height, mode, composer, inlinePlayer, playback, onPlayback, busy, error } = props;
  const { ffmpeg } = useServices();
  const state = reviewStateOf(shot);
  const inner = Math.max(20, width - 2);
  const preview = previewHeight(height);
  const heading = shot.sequence ? `${shot.sequence} · ${shot.slug}` : shot.slug;
  const canPlay = shot.isMovie && shot.videoPath !== null;
  const commentCount = shot.review?.comments.length ?? 0;
  const chapters = shot.chapters.slice(0, 3);

  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} paddingX={1} overflow="hidden">
      {mode === "page" ? (
        <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
          <Text color={theme.blue}>‹ Stream</Text>
          <Text color={theme.muted}>
            {position.index} / {position.count}
            {"  "}
            <Text color={theme.blue}>‹</Text> older · newer <Text color={theme.blue}>›</Text>
          </Text>
        </Box>
      ) : (
        <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
          <Text color={theme.muted}>Detail</Text>
          <Text color={theme.muted}>
            {position.index} / {position.count}
          </Text>
        </Box>
      )}
      <Box width={inner} height={preview} justifyContent="center" flexShrink={0}>
        {inlinePlayer && canPlay ? (
          <MoviePlayer
            videoPath={shot.videoPath!}
            posterPath={shot.path}
            posterVersion={shot.mtimeMs}
            width={inner}
            height={preview}
            playback={playback}
            onPlayback={onPlayback}
          />
        ) : (
          <Picture src={shot.path} version={shot.mtimeMs} width={inner} height={preview} maxUpscale={8} zoom={props.zoom} label={shot.isMovie ? "▶ movie poster" : "still"} />
        )}
      </Box>
      {inlinePlayer && canPlay ? (
        <ProgressBar
          positionMs={playback.positionMs}
          durationMs={playback.durationMs ?? shot.durationMs}
          chapters={shot.chapters}
          width={inner}
          playing={playback.playing}
        />
      ) : null}
      <Box height={1} width={inner} justifyContent="space-between" flexShrink={0}>
        <Text bold wrap="truncate">
          {truncate(heading, Math.max(8, inner - 24))}
        </Text>
        <Text>
          {shot.isMovie ? (
            <>
              <MovieBadge duration={durationLabel(shot.durationMs)} />
              <Text> </Text>
            </>
          ) : null}
          <ReviewBadge state={state} stale={shot.review?.isStale} />
        </Text>
      </Box>
      <Line width={inner}>
        <Text color={theme.muted} wrap="truncate">
          {truncate(shot.description || shot.fileName, inner)}
        </Text>
      </Line>
      {shot.isMovie ? (
        <Box flexDirection="column" width={inner} flexShrink={0}>
          <Line width={inner}>
          <Text wrap="truncate">
            <Text color={canPlay ? theme.green : theme.faint} bold>
              {" p "}
            </Text>
            <Text color={canPlay ? theme.text : theme.faint}>{inlinePlayer ? "Hide player" : "Play in tray"}</Text>
            <Text color={canPlay ? theme.blue : theme.faint} bold>
              {"   O "}
            </Text>
            <Text color={canPlay ? theme.text : theme.faint}>Open movie</Text>
            {!ffmpeg.ffmpeg && canPlay ? <Text color={theme.amber}>   ffmpeg missing · install to play here</Text> : null}
          </Text>
          </Line>
          {shot.videoFileName && !shot.videoPath ? (
            <Line width={inner}>
              <Text color={theme.amber}>Video missing on disk</Text>
            </Line>
          ) : !shot.videoFileName ? (
            <Line width={inner}>
              <Text color={theme.muted}>Poster only — no video path in manifest</Text>
            </Line>
          ) : null}
          {chapters.length > 0 ? (
            <Box flexDirection="column" width={inner} flexShrink={0}>
              <Line width={inner}>
                <SectionLabel>CHAPTERS</SectionLabel>
              </Line>
              {chapters.map((chapter, index) => (
                <Line key={`${chapter.slug ?? index}`} width={inner}>
                  <Text wrap="truncate">
                    <Text color={theme.purple}>{chapterTimeLabel(chapter.tMs).padStart(6)}</Text>
                    {"  "}
                    {truncate(chapter.title ?? chapter.slug ?? "chapter", inner - 8)}
                  </Text>
                </Line>
              ))}
              {shot.chapters.length > chapters.length ? (
                <Line width={inner}>
                  <Text color={theme.faint}>       … {shot.chapters.length - chapters.length} more</Text>
                </Line>
              ) : null}
            </Box>
          ) : null}
        </Box>
      ) : null}
      <Rule width={inner} />
      <Box height={1} width={inner} justifyContent="space-between" flexShrink={0}>
        <SectionLabel>FEEDBACK {commentCount}</SectionLabel>
        {busy ? <Text color={theme.muted}>Saving review…</Text> : error ? <Text color={theme.red}>{truncate(error, inner - 12)}</Text> : null}
      </Box>
      <Box flexShrink={0} width={inner}>
        <CommentList shot={shot} width={inner} max={composer ? 2 : 3} />
      </Box>
      {composer ? (
        <Box flexShrink={0} width={inner}>
          <TextInput placeholder="Share feedback…" width={inner} onSubmit={props.onComposerSubmit} onCancel={props.onComposerCancel} submitLabel="Send Feedback" />
        </Box>
      ) : (
        <Line width={inner}>
          <Text>
            <Text color={theme.blue} bold>
              {" c "}
            </Text>
            <Text color={theme.muted}>Send Feedback</Text>
            <Text color={theme.green} bold>
              {"   s "}
            </Text>
            <Text color={theme.muted}>Seen</Text>
            <Text color={theme.purple} bold>
              {"   f "}
            </Text>
            <Text color={theme.muted}>Full screen</Text>
            <Text color={theme.blue} bold>
              {"   +/- "}
            </Text>
            <Text color={theme.muted}>zoom</Text>
          </Text>
        </Line>
      )}
      <Line width={inner}>
        <Rule width={inner} />
      </Line>
      <Box flexDirection="column" width={inner} overflow="hidden" flexGrow={1} flexShrink={100} minHeight={0}>
        <MetaRow label="Tree" value={shot.worktree} width={inner} />
        <MetaRow label="Feature" value={shot.feature} width={inner} />
        <MetaRow label="File" value={shot.fileName} width={inner} />
        {shot.isMovie ? <MetaRow label="Kind" value="Movie" width={inner} /> : null}
        {shot.isMovie ? <MetaRow label="Video" value={shot.videoFileName ?? "—"} width={inner} /> : null}
        {shot.isMovie ? <MetaRow label="Duration" value={durationLabel(shot.durationMs) ?? "—"} width={inner} /> : null}
        {shot.isMovie && shot.source ? <MetaRow label="Source" value={shot.source} width={inner} /> : null}
        {shot.isMovie && shot.chapters.length > 0 ? <MetaRow label="Chapters" value={String(shot.chapters.length)} width={inner} /> : null}
        <MetaRow label="Time" value={isoDateTime(shot.capturedAt)} width={inner} />
        {shot.url ? <MetaRow label="URL" value={shot.url} width={inner} /> : null}
        {shot.runId ? <MetaRow label="Run" value={shot.runId} width={inner} /> : null}
        {shot.status ? <MetaRow label="Execution" value={shot.status.charAt(0).toUpperCase() + shot.status.slice(1)} width={inner} /> : null}
        <MetaRow label="Review" value={state === "seen" ? "Seen" : "Unseen"} width={inner} />
        <MetaRow label="Path" value={path.dirname(shot.path)} width={inner} />
      </Box>
    </Box>
  );
}
