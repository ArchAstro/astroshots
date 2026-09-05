/**
 * Reserves a cell box for an image and hands it to the graphics layer. When
 * graphics are unavailable it shows a labeled placeholder instead.
 */
import { Box, Text, type DOMElement } from "ink";
import { useEffect, useRef, useState } from "react";

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
}

export function Picture({ src, version = 0, width, height, label, border = false, z }: PictureProps) {
  const { layer, capabilities } = useServices();
  const ref = useRef<DOMElement>(null);
  const handleRef = useRef<ImageHandle | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const enabled = capabilities.graphics === "kitty";

  useEffect(() => {
    if (!enabled) return;
    const handle = layer.register({ src, version, z });
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
  }, [enabled, layer]);

  useEffect(() => {
    handleRef.current?.setSource(src, version);
    setFailure(null);
  }, [src, version]);

  useEffect(() => {
    if (!enabled || !handleRef.current) return;
    const id = handleRef.current.id;
    const timer = setInterval(() => {
      const problem = layer.failure(id);
      if (problem !== failure) setFailure(problem);
    }, 400);
    timer.unref();
    return () => clearInterval(timer);
  }, [enabled, layer, failure]);

  const innerWidth = Math.max(1, width - (border ? 2 : 0));
  const innerHeight = Math.max(1, height - (border ? 2 : 0));
  const placeholder = !enabled || !src || failure;
  const caption = failure
    ? truncate(`Preview unavailable · ${failure}`, innerWidth)
    : !src
      ? "No image"
      : truncate(label ?? "Preview needs Kitty graphics", innerWidth);

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
      {placeholder ? (
        <Box flexShrink={0} width={innerWidth} height={innerHeight} alignItems="center" justifyContent="center">
          <Text color={theme.muted}>{caption}</Text>
        </Box>
      ) : null}
    </Box>
  );
}
