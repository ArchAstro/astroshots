/**
 * Plays a movie into a cell box using ffmpeg frames and the graphics layer.
 * The poster stays underneath until the first frame lands.
 */
import { Box, Text, type DOMElement } from "ink";
import { useEffect, useRef, useState } from "react";

import type { FrameHandle } from "../terminal/image-layer.js";
import { FramePlayer, probeVideo, type VideoInfo } from "../video/ffmpeg.js";
import { useServices } from "./context.js";
import { Picture } from "./picture.js";
import { theme } from "./theme.js";

export interface PlaybackState {
  playing: boolean;
  positionMs: number;
  durationMs: number | null;
  /** Bumped by the parent to request a seek to `positionMs`. */
  seekToken: number;
  error: string | null;
  ended: boolean;
}

export interface MoviePlayerProps {
  videoPath: string;
  posterPath: string;
  posterVersion?: number;
  width: number;
  height: number;
  playback: PlaybackState;
  onPlayback: (patch: Partial<PlaybackState>) => void;
}

const MAX_FRAME_WIDTH = 1024;

export function MoviePlayer({ videoPath, posterPath, posterVersion = 0, width, height, playback, onPlayback }: MoviePlayerProps) {
  const { layer, capabilities, ffmpeg } = useServices();
  const stageRef = useRef<DOMElement>(null);
  const handleRef = useRef<FrameHandle | null>(null);
  const playerRef = useRef<FramePlayer | null>(null);
  const [info, setInfo] = useState<VideoInfo | null>(null);
  const [hasFrame, setHasFrame] = useState(false);
  const enabled = capabilities.graphics === "kitty" || capabilities.graphics === "herdr";
  const positionRef = useRef(playback.positionMs);
  positionRef.current = playback.positionMs;

  useEffect(() => {
    if (!enabled) return;
    const handle = layer.registerFrames({ z: 1 });
    handle.setNode(stageRef.current);
    handleRef.current = handle;
    layer.scheduleFlush();
    return () => {
      handle.unregister();
      handleRef.current = null;
    };
  }, [enabled, layer]);

  useEffect(() => {
    let cancelled = false;
    setInfo(null);
    void probeVideo(videoPath, ffmpeg).then((result) => {
      if (cancelled) return;
      setInfo(result);
      if (result?.durationMs && !playback.durationMs) onPlayback({ durationMs: result.durationMs });
      if (!result) onPlayback({ error: ffmpeg.ffmpeg ? "Could not read this movie" : "Install ffmpeg to play movies" });
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [videoPath]);

  // Start/stop/seek the decoder.
  useEffect(() => {
    playerRef.current?.stop();
    playerRef.current = null;
    if (!enabled || !info || !playback.playing) return;
    if (!ffmpeg.ffmpeg) {
      onPlayback({ playing: false, error: "Install ffmpeg to play movies (brew install ffmpeg)" });
      return;
    }
    const bounds = {
      width: Math.min(MAX_FRAME_WIDTH, width * capabilities.cellWidth),
      height: Math.min(Math.round((MAX_FRAME_WIDTH * 3) / 4), height * capabilities.cellHeight),
    };
    let lastReport = 0;
    const player = new FramePlayer({
      videoPath,
      bounds,
      sourceSize: { width: info.width, height: info.height },
      startMs: positionRef.current,
      ffmpegPath: ffmpeg.ffmpeg,
      onFrame: (frame) => {
        handleRef.current?.pushFrame({ png: frame.png, width: frame.width, height: frame.height });
        if (!hasFrame) setHasFrame(true);
        if (frame.tMs - lastReport >= 250) {
          lastReport = frame.tMs;
          onPlayback({ positionMs: frame.tMs });
        }
      },
      onEnd: () => onPlayback({ playing: false, ended: true, positionMs: playback.durationMs ?? positionRef.current }),
      onError: (error) => onPlayback({ playing: false, error: error.message }),
    });
    playerRef.current = player;
    player.start();
    return () => {
      player.stop();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, info, playback.playing, playback.seekToken, width, height, videoPath]);

  useEffect(() => () => handleRef.current?.clearFrame(), []);

  return (
    <Box ref={stageRef} width={width} height={height} flexShrink={0} alignItems="center" justifyContent="center" overflow="hidden">
      {!hasFrame ? <Picture src={posterPath} version={posterVersion} width={width} height={height} label="▶ movie" /> : null}
      {!enabled ? (
        <Box position="absolute">
          <Text color={theme.muted}>Poster shown · press O to open the movie</Text>
        </Box>
      ) : null}
    </Box>
  );
}

export function formatClock(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

export function ProgressBar({
  positionMs,
  durationMs,
  chapters,
  width,
  playing,
}: {
  positionMs: number;
  durationMs: number | null;
  chapters: Array<{ tMs?: number }>;
  width: number;
  playing: boolean;
}) {
  const clock = `${formatClock(positionMs)} / ${durationMs ? formatClock(durationMs) : "--:--"}`;
  const barWidth = Math.max(4, width - clock.length - 6);
  const ratio = durationMs ? Math.min(1, positionMs / durationMs) : 0;
  const filled = Math.round(ratio * barWidth);
  const markers = new Set(
    chapters
      .filter((chapter) => chapter.tMs !== undefined && durationMs)
      .map((chapter) => Math.min(barWidth - 1, Math.round((chapter.tMs! / durationMs!) * barWidth))),
  );
  let bar = "";
  for (let index = 0; index < barWidth; index += 1) {
    if (index === filled) bar += "●";
    else if (markers.has(index)) bar += "┼";
    else bar += index < filled ? "━" : "─";
  }
  return (
    <Box flexShrink={0} width={width} height={1}>
      <Text color={playing ? theme.green : theme.muted}>{playing ? "▶ " : "⏸ "}</Text>
      <Text color={theme.purple}>{bar}</Text>
      <Text color={theme.muted}> {clock}</Text>
    </Box>
  );
}
