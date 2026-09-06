/**
 * Full-screen review: header, image stage, and the feedback rail.
 * Pages over run siblings, oldest → newest.
 */
import { Box, Text } from "ink";

import type { Shot } from "../data/model.js";
import { CommentList } from "./detail.js";
import { Line, ReviewBadge, Rule, SectionLabel } from "./chrome.js";
import { MoviePlayer, ProgressBar, type PlaybackState } from "./movie-player.js";
import { Picture } from "./picture.js";
import { reviewStateOf } from "./selectors.js";
import { TextInput } from "./text-input.js";
import { abbreviatedDateTime, theme, truncate } from "./theme.js";

export const RAIL_WIDTH = 42;

export interface ReviewTakeoverProps {
  shot: Shot;
  position: { index: number; count: number };
  width: number;
  height: number;
  composer: boolean;
  onComposerSubmit: (text: string) => void;
  onComposerCancel: () => void;
  playback: PlaybackState;
  onPlayback: (patch: Partial<PlaybackState>) => void;
  playing: boolean;
  busy: boolean;
  error: string | null;
  /** 1 shows the whole image; above magnifies (crops in). */
  zoom: number;
  /** Pan center as a fraction of the image, [0,1]. */
  panX: number;
  panY: number;
}

export function ReviewTakeover(props: ReviewTakeoverProps) {
  const { shot, position, width, height, composer, playback, onPlayback, playing, busy, error, zoom, panX, panY } = props;
  const railWidth = width >= 100 ? RAIL_WIDTH : Math.max(28, Math.floor(width * 0.38));
  const stageWidth = Math.max(10, width - railWidth - 1);
  const headerHeight = 3;
  const footerHeight = 1;
  const stageHeight = Math.max(4, height - headerHeight - footerHeight);
  const canPlay = shot.isMovie && shot.videoPath !== null;
  const showPlayer = canPlay && (playing || playback.positionMs > 0);
  const meta = [shot.worktree, shot.feature, shot.url ?? "", abbreviatedDateTime(shot.capturedAt)].filter(Boolean).join("  ·  ");
  const state = reviewStateOf(shot);
  const railInner = railWidth - 2;
  const imageHeight = showPlayer ? stageHeight - 1 : stageHeight;

  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height}>
      <Box flexShrink={0} height={1} width={width} paddingX={1} justifyContent="space-between">
        <Text wrap="truncate">
          <Text bold>{truncate(shot.title, Math.max(8, width - 40))}</Text>
          {shot.sequence ? (
            <Text color={theme.muted}>
              {"  "}
              <Text backgroundColor={theme.selection}> {shot.sequence} </Text>
            </Text>
          ) : null}
        </Text>
        <Text color={theme.muted}>
          <Text color={theme.blue}>‹</Text> {position.index} / {position.count} <Text color={theme.blue}>›</Text>
          {"   esc close"}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} width={width} paddingX={1}>
        <Text color={theme.muted} wrap="truncate">
          {truncate(meta, width - 2)}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} width={width} paddingX={1}>
        <Rule width={width - 2} />
      </Box>
      <Box flexShrink={0} flexDirection="row" width={width} height={stageHeight}>
        <Box flexShrink={0} flexDirection="column" width={stageWidth} height={stageHeight} alignItems="center" justifyContent="center">
          {showPlayer ? (
            <MoviePlayer
              videoPath={shot.videoPath!}
              posterPath={shot.path}
              posterVersion={shot.mtimeMs}
              width={stageWidth}
              height={imageHeight}
              playback={playback}
              onPlayback={onPlayback}
            />
          ) : (
            <Picture src={shot.path} version={shot.mtimeMs} width={stageWidth} height={imageHeight} maxUpscale={8} zoom={zoom} panX={panX} panY={panY} label={shot.isMovie ? "▶ movie poster" : "Screenshot unavailable"} />
          )}
          {showPlayer ? (
            <ProgressBar
              positionMs={playback.positionMs}
              durationMs={playback.durationMs ?? shot.durationMs}
              chapters={shot.chapters}
              width={stageWidth - 2}
              playing={playback.playing}
            />
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
          <Box flexShrink={0} height={1} justifyContent="space-between" width={railInner}>
            <Text bold>Review</Text>
            <ReviewBadge state={state} stale={shot.review?.isStale} />
          </Box>
          {shot.review?.isStale ? (
            <Box flexShrink={0} width={railInner} height={2}>
              <Text color={theme.amber} wrap="wrap">
                A newer image was captured since this was seen.
              </Text>
            </Box>
          ) : null}
          <Box flexShrink={0} width={railInner} height={Math.min(3, Math.max(1, Math.ceil((shot.description || shot.fileName).length / railInner)))} overflow="hidden">
            <Text color={theme.muted} wrap="wrap">
              {truncate(shot.description || shot.fileName, railInner * 3)}
            </Text>
          </Box>
          <Line width={railInner}>
            <Rule width={railInner} />
          </Line>
          <Box flexShrink={0} height={1} justifyContent="space-between" width={railInner}>
            <SectionLabel>FEEDBACK</SectionLabel>
            <Text color={theme.muted}>{shot.review?.comments.length ?? 0}</Text>
          </Box>
          <Box flexDirection="column" flexGrow={1} flexShrink={100} minHeight={0} overflow="hidden" width={railInner}>
            <CommentList shot={shot} width={railInner} max={Math.max(2, Math.floor((stageHeight - 12) / 3))} />
          </Box>
          <Line width={railInner}>
            <Rule width={railInner} />
          </Line>
          {busy ? (
            <Line width={railInner}>
              <Text color={theme.muted}>Saving review…</Text>
            </Line>
          ) : null}
          {error ? (
            <Line width={railInner}>
              <Text color={theme.red}>{truncate(error, railInner)}</Text>
            </Line>
          ) : null}
          {composer ? (
            <TextInput placeholder="Share feedback…" width={railInner} onSubmit={props.onComposerSubmit} onCancel={props.onComposerCancel} submitLabel="Send Feedback" />
          ) : (
            <Box flexDirection="column" flexShrink={0}>
              <Line width={railInner}>
                <Text>
                  <Text color={theme.blue} bold>
                    {" c "}
                  </Text>
                  <Text>Send Feedback</Text>
                  <Text color={theme.green} bold>
                    {"   s "}
                  </Text>
                  <Text>Seen</Text>
                </Text>
              </Line>
              <Line width={railInner}>
                <Text color={theme.muted} wrap="truncate">
                  {zoom > 1 ? "← ↑ → ↓ pan · +/- zoom · 0 reset · esc close" : canPlay ? "← → page · space play · +/- zoom · [ ] chapter" : "← → page · +/- zoom · c send · s seen"}
                </Text>
              </Line>
            </Box>
          )}
        </Box>
      </Box>
    </Box>
  );
}
