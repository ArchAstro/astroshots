/**
 * Shows an image in a reserved cell box. With the Kitty protocol the graphics
 * layer paints real pixels; otherwise (mosh, tmux, herdr, or a plain terminal)
 * it renders the picture as truecolor half-block text, which survives any
 * transport. Falls back to a labeled placeholder when nothing can be drawn.
 */
import { Box, Text, type DOMElement } from "ink";
import { useEffect, useRef, useState } from "react";

import { rgbToHalfBlockLines } from "../images/halfblocks.js";
import type { ImageHandle } from "../terminal/image-layer.js";
import { useServices } from "./context.js";
import { theme, truncate } from "./theme.js";

export interface PictureProps {
  src: string | null;
  /** Changes when the file's bytes change (mtime), so a re-capture refreshes. */
  version?: number;
  width: number;
  height: number;
  /** Text shown when the terminal cannot draw pictures. */
  label?: string;
  border?: boolean;
  z?: number;
  /** Allow the image to scale up to this multiple of native to fill its box (default 1 = never upscale). */
  maxUpscale?: number;
  /** Magnification: 1 shows the whole image; >1 crops in. */
  zoom?: number;
  /** Pan center as a fraction of the image, [0,1]. */
  panX?: number;
  panY?: number;
}

export function Picture({ src, version = 0, width, height, label, border = false, z, maxUpscale = 1, zoom = 1, panX = 0.5, panY = 0.5 }: PictureProps) {
  const { layer, service, capabilities } = useServices();
  const ref = useRef<DOMElement>(null);
  const handleRef = useRef<ImageHandle | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const mode = capabilities.graphics;
  // Both Kitty and herdr paint through the out-of-band layer.
  const kitty = mode === "kitty" || mode === "herdr";

  const innerWidth = Math.max(1, width - (border ? 2 : 0));
  const innerHeight = Math.max(1, height - (border ? 2 : 0));

  // ---- Kitty: reserve the box and let the out-of-band layer paint it. ----
  useEffect(() => {
    if (!kitty) return;
    const handle = layer.register({ src, version, z, maxUpscale, zoom });
    handleRef.current = handle;
    handle.setNode(ref.current);
    // Ink already painted this commit before the effect ran; measure now so
    // the picture is requested without waiting for an unrelated re-render.
    layer.scheduleFlush();
    return () => {
      handle.unregister();
      handleRef.current = null;
    };
    // The layer handle lives for the component; src changes go through setSource.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kitty, layer]);

  useEffect(() => {
    if (!kitty) return;
    handleRef.current?.setSource(src, version);
    setFailure(null);
  }, [kitty, src, version]);

  useEffect(() => {
    if (!kitty) return;
    handleRef.current?.setView({ zoom, panX, panY, maxUpscale });
  }, [kitty, zoom, panX, panY, maxUpscale]);

  useEffect(() => {
    if (!kitty || !handleRef.current) return;
    const id = handleRef.current.id;
    const timer = setInterval(() => {
      const problem = layer.failure(id);
      if (problem !== failure) setFailure(problem);
    }, 400);
    timer.unref();
    return () => clearInterval(timer);
  }, [kitty, layer, failure]);

  // ---- Half-blocks: decode to a tiny RGB buffer and draw two pixels per cell. ----
  const [lines, setLines] = useState<string[] | null>(null);
  useEffect(() => {
    if (mode !== "halfblocks") return;
    let cancelled = false;
    setFailure(null);
    if (!src) {
      setLines(null);
      return;
    }
    // One cell is one pixel wide and two pixels tall.
    const target = { width: innerWidth, height: innerHeight * 2 };
    void service.prepare(src, target, "rgb").then(
      (prepared) => {
        if (cancelled) return;
        setLines(rgbToHalfBlockLines({ width: prepared.width, height: prepared.height, rgb: prepared.data }));
      },
      (error: unknown) => {
        if (cancelled) return;
        setLines(null);
        setFailure(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [mode, service, src, version, innerWidth, innerHeight]);

  const art = mode === "halfblocks" && src && lines && lines.length > 0 && !failure;
  const placeholder = !art && (!kitty || !src || failure);
  const caption = failure
    ? truncate(`Preview unavailable · ${failure}`, innerWidth)
    : !src
      ? "No image"
      : mode === "halfblocks"
        ? "Rendering…"
        : truncate(label ?? "Preview needs a graphics terminal", innerWidth);

  return (
    <Box
      ref={ref}
      width={width}
      height={height}
      flexShrink={0}
      borderStyle={border ? "round" : undefined}
      borderColor={theme.faint}
      alignItems="center"
      justifyContent="center"
      overflow="hidden"
    >
      {art ? (
        <Box flexDirection="column" flexShrink={0} alignItems="center">
          {lines!.slice(0, innerHeight).map((line, index) => (
            <Text key={index} wrap="truncate-end">
              {line}
            </Text>
          ))}
        </Box>
      ) : placeholder ? (
        <Box flexShrink={0} width={innerWidth} height={innerHeight} alignItems="center" justifyContent="center">
          <Text color={theme.muted}>{caption}</Text>
        </Box>
      ) : null}
    </Box>
  );
}
