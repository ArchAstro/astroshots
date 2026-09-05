/**
 * Bridges Ink's layout tree to kitty graphics placements.
 *
 * Components register a source path and the DOM element that reserves cells
 * for the picture. After every Ink frame the layer measures each element,
 * fits the picture into that box, transmits bytes the terminal has not seen
 * yet, and (re)places every visible image. Placements share the frame's
 * synchronized-update block so text and pictures land together.
 */
import { measureElement, type DOMElement } from "ink";

import { fitInside } from "../images/png.js";
import type { ImageService, PreparedImage } from "../images/service.js";
import {
  RESTORE_CURSOR,
  SAVE_CURSOR,
  cursorTo,
  encodeDelete,
  encodePlace,
  encodeTransmit,
} from "./kitty.js";
import type { HerdrSink } from "./herdr.js";
import type { TerminalCapabilities } from "./probe.js";

export interface ImageHandle {
  readonly id: number;
  setNode(node: DOMElement | null): void;
  /** `version` (for example the file's mtime) forces a fresh decode when the bytes change. */
  setSource(src: string | null, version?: number): void;
  /** `zoom` scales within [native..fill]; `maxUpscale` caps how far past native it may grow. */
  setZoom(zoom: number, maxUpscale?: number): void;
  unregister(): void;
}

/** A picture that changes every few milliseconds (movie playback). */
export interface FrameHandle {
  readonly id: number;
  setNode(node: DOMElement | null): void;
  /** Transmit and show a new PNG frame immediately. */
  pushFrame(frame: { png: Buffer; width: number; height: number }): void;
  /** Drop the current frame (the poster shows through again). */
  clearFrame(): void;
  unregister(): void;
}

interface Entry {
  id: number;
  src: string | null;
  version: number;
  z: number;
  /** How far past native size the image may scale (1 = never upscale). */
  maxUpscale: number;
  /** User zoom within the allowed range (1 = as large as allowed). */
  zoom: number;
  node: DOMElement | null;
  ready: PreparedImage | null;
  requestedKey: string | null;
  failed: string | null;
  /** Live frame state, for frame entries. */
  frame: { imageId: number; width: number; height: number } | null;
  /** Last frame bytes, kept so herdr can re-place on a resize. */
  frameData: Buffer | null;
  kind: "file" | "frames";
}

export interface Placement {
  placementId: number;
  imageId: number;
  col: number;
  row: number;
  cols: number;
  rows: number;
  z: number;
}

export interface ImageLayerOptions {
  capabilities: TerminalCapabilities;
  service: ImageService;
  /** Writes straight to the terminal, bypassing Ink (used for async readiness). */
  write: (data: string) => void;
  /** Called when an image finished preparing and the screen should refresh. */
  onReady?: () => void;
  onError?: (src: string, error: Error) => void;
  /** Diagnostics sink (enabled by ASTROSHOT_REVIEW_DEBUG). */
  onDebug?: (message: string) => void;
  /** When present, images are placed through herdr's socket API instead of Kitty escapes. */
  herdr?: HerdrSink;
  maxTransmitted?: number;
}

interface TransmittedImage {
  id: number;
  key: string;
  lastUsed: number;
}

export class ImageLayer {
  private readonly entries = new Map<number, Entry>();
  private readonly transmitted = new Map<string, TransmittedImage>();
  private lastPlacements = new Map<number, Placement>();
  private nextEntryId = 1;
  private nextImageId = 1000 + Math.floor(Math.random() * 100_000);
  private tick = 0;
  private resync = true;
  private flushScheduled = false;
  private readonly capabilities: TerminalCapabilities;
  private readonly service: ImageService;
  private readonly write: (data: string) => void;
  private readonly onReady?: () => void;
  private readonly onError?: (src: string, error: Error) => void;
  private readonly onDebug?: (message: string) => void;
  private readonly herdr?: HerdrSink;
  private readonly herdrLayers = new Map<number, string>();
  private herdrGeneration = -1;
  private readonly maxTransmitted: number;
  private lastSummary = "";
  /** Row offset of the live region's first line, 1-based screen row minus 1. */
  originRow = 0;
  originCol = 0;

  constructor(options: ImageLayerOptions) {
    this.capabilities = options.capabilities;
    this.service = options.service;
    this.write = options.write;
    this.onReady = options.onReady;
    this.onError = options.onError;
    this.onDebug = options.onDebug;
    this.herdr = options.herdr;
    this.maxTransmitted = options.maxTransmitted ?? 48;
  }

  get enabled(): boolean {
    return this.capabilities.graphics === "kitty" || Boolean(this.herdr);
  }

  private herdrLayerId(entryId: number): string {
    return `astro-${entryId}`;
  }

  register(options: { src: string | null; version?: number; z?: number; maxUpscale?: number; zoom?: number }): ImageHandle {
    const id = this.nextEntryId++;
    const entry: Entry = {
      id,
      src: options.src,
      version: options.version ?? 0,
      z: options.z ?? 0,
      maxUpscale: options.maxUpscale ?? 1,
      zoom: options.zoom ?? 1,
      node: null,
      ready: null,
      requestedKey: null,
      failed: null,
      frame: null,
      frameData: null,
      kind: "file",
    };
    this.entries.set(id, entry);
    return {
      id,
      setNode: (node) => {
        entry.node = node;
      },
      setSource: (src, version = 0) => {
        if (entry.src === src && entry.version === version) return;
        entry.src = src;
        entry.version = version;
        entry.ready = null;
        entry.requestedKey = null;
        entry.failed = null;
        this.scheduleFlush();
      },
      setZoom: (zoom, maxUpscale) => {
        if (entry.zoom === zoom && (maxUpscale === undefined || entry.maxUpscale === maxUpscale)) return;
        entry.zoom = zoom;
        if (maxUpscale !== undefined) entry.maxUpscale = maxUpscale;
        this.scheduleFlush();
      },
      unregister: () => {
        this.entries.delete(id);
        if (this.herdr) {
          this.herdr.clear(this.herdrLayerId(id));
          this.herdrLayers.delete(id);
        }
        this.scheduleFlush();
      },
    };
  }

  registerFrames(options: { z?: number } = {}): FrameHandle {
    const id = this.nextEntryId++;
    const entry: Entry = {
      id,
      src: null,
      version: 0,
      z: options.z ?? 1,
      maxUpscale: 1,
      zoom: 1,
      node: null,
      ready: null,
      requestedKey: null,
      failed: null,
      frame: null,
      frameData: null,
      kind: "frames",
    };
    this.entries.set(id, entry);
    const dropFrame = (): string => {
      if (!entry.frame) return "";
      const output = encodeDelete({ kind: "image", id: entry.frame.imageId });
      entry.frame = null;
      return output;
    };
    return {
      id,
      setNode: (node) => {
        entry.node = node;
      },
      pushFrame: (frame) => {
        if (!this.enabled) return;
        const imageId = this.nextImageId++;
        entry.frame = { imageId, width: frame.width, height: frame.height };
        entry.frameData = frame.png;
        if (this.herdr) {
          const placement = this.placementFor(entry);
          if (placement) {
            this.herdr.set(this.herdrLayerId(entry.id), frame.png, frame.width, frame.height, placement);
            this.herdrLayers.set(entry.id, `frame-${imageId}`);
          }
          return;
        }
        const previous = { imageId: entry.frame.imageId };
        let output = encodeTransmit({ id: imageId, format: 100, data: frame.png });
        const placement = this.placementFor(entry);
        if (placement) {
          output += SAVE_CURSOR + this.placeCommand(placement) + RESTORE_CURSOR;
          this.lastPlacements.set(entry.id, placement);
        }
        void previous;
        this.write(`\x1b[?2026h${output}\x1b[?2026l`);
      },
      clearFrame: () => {
        entry.frame = null;
        entry.frameData = null;
        if (this.herdr) {
          this.herdr.clear(this.herdrLayerId(entry.id));
          this.herdrLayers.delete(entry.id);
          return;
        }
        const output = dropFrame();
        this.lastPlacements.delete(entry.id);
        if (output) this.write(output);
      },
      unregister: () => {
        entry.frame = null;
        entry.frameData = null;
        if (this.herdr) {
          this.herdr.clear(this.herdrLayerId(entry.id));
          this.herdrLayers.delete(entry.id);
          this.entries.delete(id);
          return;
        }
        const output = dropFrame();
        this.entries.delete(id);
        this.lastPlacements.delete(entry.id);
        if (output) this.write(output);
      },
    };
  }

  /** Where a frame entry's current picture lands, given the live layout. */
  private placementFor(entry: Entry): Placement | null {
    if (!entry.node || !entry.frame) return null;
    const box = measureElement(entry.node);
    if (box.width <= 0 || box.height <= 0) return null;
    const cellWidth = this.capabilities.cellWidth;
    const cellHeight = this.capabilities.cellHeight;
    const targetPx = { width: box.width * cellWidth, height: box.height * cellHeight };
    const fitted = fitInside({ width: entry.frame.width, height: entry.frame.height }, targetPx);
    const cols = Math.min(box.width, Math.max(1, Math.round(fitted.width / cellWidth)));
    const rows = Math.min(box.height, Math.max(1, Math.round(fitted.height / cellHeight)));
    return {
      placementId: entry.id,
      imageId: entry.frame.imageId,
      col: box.x + Math.floor((box.width - cols) / 2),
      row: box.y + Math.floor((box.height - rows) / 2),
      cols,
      rows,
      z: entry.z,
    };
  }

  private placeCommand(placement: Placement): string {
    return (
      cursorTo(placement.row + 1 + this.originRow, placement.col + 1 + this.originCol) +
      encodePlace({
        id: placement.imageId,
        placementId: placement.placementId,
        cols: placement.cols,
        rows: placement.rows,
        z: placement.z,
      })
    );
  }

  /** Prepared image for an entry, if any (lets components show dimensions). */
  prepared(id: number): PreparedImage | null {
    return this.entries.get(id)?.ready ?? null;
  }

  failure(id: number): string | null {
    return this.entries.get(id)?.failed ?? null;
  }

  /** After a resize or screen clear, every placement must be re-sent. */
  invalidate(): void {
    this.resync = true;
    // Force herdr layers to be re-set at their new positions.
    this.herdrLayers.clear();
  }

  /**
   * Reconcile the herdr layers with the current placement set: (re)place any
   * image whose bytes or box changed, and clear layers no longer shown.
   */
  private syncHerdr(placements: Map<number, Placement>, readyByEntry: Map<number, PreparedImage>): void {
    const sink = this.herdr;
    if (!sink) return;
    // A dropped-and-restored connection loses server-side layers; re-send all.
    if (sink.generation !== this.herdrGeneration) {
      this.herdrGeneration = sink.generation;
      this.herdrLayers.clear();
    }
    for (const [entryId, placement] of placements) {
      const ready = readyByEntry.get(entryId);
      if (!ready) continue; // frame entries place themselves in pushFrame
      const signature = `${ready.key}|${placement.col},${placement.row},${placement.cols},${placement.rows},${placement.z}`;
      if (this.herdrLayers.get(entryId) === signature) continue;
      sink.set(this.herdrLayerId(entryId), ready.data, ready.width, ready.height, placement);
      this.herdrLayers.set(entryId, signature);
    }
    for (const entryId of [...this.herdrLayers.keys()]) {
      const entry = this.entries.get(entryId);
      // Keep active file placements and live frame entries; clear the rest.
      if (placements.has(entryId)) continue;
      if (entry?.kind === "frames" && entry.frame) continue;
      sink.clear(this.herdrLayerId(entryId));
      this.herdrLayers.delete(entryId);
    }
  }

  /** Escape sequences that bring the terminal in line with the current layout. */
  render(): string {
    if (!this.enabled) return "";
    this.tick += 1;
    const placements = new Map<number, Placement>();
    const readyByEntry = new Map<number, PreparedImage>();
    let output = "";
    const cellWidth = this.capabilities.cellWidth;
    const cellHeight = this.capabilities.cellHeight;

    for (const entry of this.entries.values()) {
      if (entry.kind === "frames") {
        // Frame entries place themselves in pushFrame; on herdr they re-place
        // there too. Only the kitty escape path needs them re-emitted here.
        if (!this.herdr) {
          const placement = this.placementFor(entry);
          if (placement) placements.set(entry.id, placement);
        }
        continue;
      }
      if (!entry.src || !entry.node) continue;
      const box = measureElement(entry.node);
      if (box.width <= 0 || box.height <= 0) continue;
      const boxPx = { width: box.width * cellWidth, height: box.height * cellHeight };
      // Prepare above the cell box so the compositor only ever downscales
      // (downscaling stays sharp; upscaling blurs). herdr may render the box at
      // more physical pixels than its reported cell size implies, so oversample.
      const supersample = this.herdr ? 2 : 1;
      const targetPx = { width: boxPx.width * supersample, height: boxPx.height * supersample };
      const requestKey = `${entry.src}|${entry.version}|${targetPx.width}x${targetPx.height}`;
      if (entry.requestedKey !== requestKey) {
        entry.requestedKey = requestKey;
        entry.failed = null;
        void this.service.prepare(entry.src, targetPx).then(
          (prepared) => {
            if (entry.requestedKey !== requestKey) return;
            entry.ready = prepared;
            this.scheduleFlush();
            this.onReady?.();
          },
          (error: unknown) => {
            if (entry.requestedKey !== requestKey) return;
            entry.failed = error instanceof Error ? error.message : String(error);
            this.onError?.(entry.src ?? "", error instanceof Error ? error : new Error(String(error)));
            this.onReady?.();
          },
        );
      }
      const ready = entry.ready;
      if (!ready) continue;
      readyByEntry.set(entry.id, ready);

      // On-screen size: scale to fit the box, allowing upscale up to
      // `maxUpscale`× native, then the user's zoom, capped at filling the box.
      const containScale = Math.min(boxPx.width / ready.width, boxPx.height / ready.height);
      const nativeCap = Math.min(containScale, Math.max(1, entry.maxUpscale));
      const scale = Math.min(containScale, nativeCap * Math.max(0.1, entry.zoom));
      const cols = Math.min(box.width, Math.max(1, Math.round((ready.width * scale) / cellWidth)));
      const rows = Math.min(box.height, Math.max(1, Math.round((ready.height * scale) / cellHeight)));
      const col = box.x + Math.floor((box.width - cols) / 2);
      const row = box.y + Math.floor((box.height - rows) / 2);

      let imageId = 0;
      if (!this.herdr) {
        let image = this.transmitted.get(ready.key);
        if (!image) {
          image = { id: this.nextImageId++, key: ready.key, lastUsed: this.tick };
          this.transmitted.set(ready.key, image);
          output += encodeTransmit({
            id: image.id,
            format: 100,
            data: ready.data,
            filePath: this.capabilities.fileMedium && ready.isOriginal ? ready.path : undefined,
          });
        }
        image.lastUsed = this.tick;
        imageId = image.id;
      }
      placements.set(entry.id, { placementId: entry.id, imageId, col, row, cols, rows, z: entry.z });
    }

    // herdr composits images on named layers over the pane's text; drive its
    // socket API instead of writing Kitty escapes the multiplexer would drop.
    if (this.herdr) {
      this.syncHerdr(placements, readyByEntry);
      this.lastPlacements = placements;
      if (this.onDebug) {
        const summary = `herdr layers=${this.herdrLayers.size} placed=${placements.size}`;
        if (summary !== this.lastSummary) {
          this.lastSummary = summary;
          this.onDebug(summary);
        }
      }
      return "";
    }

    if (this.resync) {
      output += encodeDelete({ kind: "all-placements" });
    } else {
      for (const [placementId, previous] of this.lastPlacements) {
        if (!placements.has(placementId)) {
          output += encodeDelete({ kind: "placement", id: previous.imageId, placementId });
        }
      }
    }

    if (placements.size > 0) {
      output += SAVE_CURSOR;
      for (const placement of placements.values()) {
        output += this.placeCommand(placement);
      }
      output += RESTORE_CURSOR;
    }

    output += this.evictTransmitted(placements);
    this.lastPlacements = placements;
    this.resync = false;
    if (this.onDebug) {
      const summary = `entries=${this.entries.size} placed=${placements.size} transmitted=${this.transmitted.size} bytes=${output.length}`;
      if (summary !== this.lastSummary) {
        this.lastSummary = summary;
        this.onDebug(summary);
      }
    }
    return output;
  }

  private evictTransmitted(active: Map<number, Placement>): string {
    if (this.transmitted.size <= this.maxTransmitted) return "";
    const inUse = new Set([...active.values()].map((placement) => placement.imageId));
    const candidates = [...this.transmitted.values()]
      .filter((image) => !inUse.has(image.id))
      .sort((a, b) => a.lastUsed - b.lastUsed);
    let output = "";
    while (this.transmitted.size > this.maxTransmitted && candidates.length > 0) {
      const image = candidates.shift()!;
      this.transmitted.delete(image.key);
      output += encodeDelete({ kind: "image", id: image.id });
    }
    return output;
  }

  /** Write placements now, outside an Ink frame. Safe for the cursor. */
  flushNow(): void {
    this.flushScheduled = false;
    if (!this.enabled) return;
    const output = this.render();
    if (output) this.write(`\x1b[?2026h${output}\x1b[?2026l`);
  }

  scheduleFlush(): void {
    if (this.flushScheduled || !this.enabled) return;
    this.flushScheduled = true;
    setImmediate(() => {
      if (this.flushScheduled) this.flushNow();
    });
  }

  /** Remove every placement and free image data in the terminal. */
  clear(): string {
    this.lastPlacements = new Map();
    this.transmitted.clear();
    this.resync = true;
    if (this.herdr) {
      this.herdr.clearAll();
      this.herdrLayers.clear();
      return "";
    }
    return this.enabled ? encodeDelete({ kind: "all" }) : "";
  }
}
