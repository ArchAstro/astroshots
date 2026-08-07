/**
 * Truecolor ANSI → HTML paint path (same model as tui-shot terminal-html).
 * Color fidelity is a hard requirement for PTY movies.
 */
import xtermHeadless, {
  type IBufferCell,
  type Terminal as TerminalType,
} from "@xterm/headless";

const { Terminal } = xtermHeadless as unknown as {
  Terminal: typeof TerminalType;
};

export type HeadlessTerminal = TerminalType;

const ANSI_16 = [
  "#2b2f3a",
  "#f0727a",
  "#54e0a0",
  "#f0b86e",
  "#7aa2f7",
  "#b9a8ff",
  "#5bd6e0",
  "#e8e8f2",
  "#5a5a7a",
  "#ff8d94",
  "#78efb6",
  "#ffd08a",
  "#9bbcff",
  "#d1c5ff",
  "#86eaf0",
  "#ffffff",
];

function rgb(red: number, green: number, blue: number): string {
  return `#${[red, green, blue]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("")}`;
}

function paletteColor(index: number): string {
  if (index < ANSI_16.length) return ANSI_16[index]!;
  if (index < 232) {
    const value = index - 16;
    const levels = [0, 95, 135, 175, 215, 255];
    return rgb(
      levels[Math.floor(value / 36)]!,
      levels[Math.floor((value % 36) / 6)]!,
      levels[value % 6]!,
    );
  }
  const gray = 8 + (index - 232) * 10;
  return rgb(gray, gray, gray);
}

function packedRgb(value: number): string {
  return rgb((value >> 16) & 255, (value >> 8) & 255, value & 255);
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

interface CellStyle {
  foreground: string;
  background: string;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  invisible: boolean;
}

function cellStyle(
  cell: IBufferCell,
  defaultForeground: string,
  defaultBackground: string,
): CellStyle {
  let foreground = cell.isFgRGB()
    ? packedRgb(cell.getFgColor())
    : cell.isFgPalette()
      ? paletteColor(cell.getFgColor())
      : defaultForeground;
  let background = cell.isBgRGB()
    ? packedRgb(cell.getBgColor())
    : cell.isBgPalette()
      ? paletteColor(cell.getBgColor())
      : defaultBackground;
  if (cell.isInverse()) [foreground, background] = [background, foreground];
  return {
    foreground,
    background,
    bold: Boolean(cell.isBold()),
    dim: Boolean(cell.isDim()),
    italic: Boolean(cell.isItalic()),
    underline: Boolean(cell.isUnderline()),
    strike: Boolean(cell.isStrikethrough()),
    invisible: Boolean(cell.isInvisible()),
  };
}

function styleAttribute(style: CellStyle): string {
  return [
    `color:${style.invisible ? style.background : style.foreground}`,
    `background:${style.background}`,
    style.bold ? "font-weight:700" : "",
    style.dim ? "opacity:.62" : "",
    style.italic ? "font-style:italic" : "",
    style.underline && style.strike
      ? "text-decoration:underline line-through"
      : style.underline
        ? "text-decoration:underline"
        : style.strike
          ? "text-decoration:line-through"
          : "",
  ]
    .filter(Boolean)
    .join(";");
}

export interface TerminalPaintOptions {
  cols: number;
  rows: number;
  foreground: string;
  background: string;
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  padding: number;
  borderRadius: number;
  scale: number;
}

export function createHeadlessTerminal(
  cols: number,
  rows: number,
): HeadlessTerminal {
  return new Terminal({
    cols,
    rows,
    allowProposedApi: true,
    convertEol: true,
    scrollback: 0,
  });
}

export function writeTerminal(
  terminal: HeadlessTerminal,
  data: string,
): Promise<void> {
  return new Promise<void>((resolve) => terminal.write(data, resolve));
}

export function terminalPlainText(
  terminal: HeadlessTerminal,
  rows: number,
): string {
  const buffer = terminal.buffer.active;
  const lines: string[] = [];
  for (let y = 0; y < rows; y++) {
    const line = buffer.getLine(buffer.viewportY + y);
    lines.push(line?.translateToString(true) ?? "");
  }
  return lines.join("\n").trimEnd();
}

export function terminalToHtml(
  terminal: HeadlessTerminal,
  options: Pick<
    TerminalPaintOptions,
    "cols" | "rows" | "foreground" | "background"
  >,
): string {
  const buffer = terminal.buffer.active;
  const rows: string[] = [];
  for (let y = 0; y < options.rows; y++) {
    const line = buffer.getLine(buffer.viewportY + y);
    const runs: Array<{ key: string; style: CellStyle; text: string }> = [];
    for (let x = 0; x < options.cols; x++) {
      const cell = line?.getCell(x);
      if (cell?.getWidth() === 0) continue;
      const style = cell
        ? cellStyle(cell, options.foreground, options.background)
        : {
            foreground: options.foreground,
            background: options.background,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strike: false,
            invisible: false,
          };
      const key = JSON.stringify(style);
      const text = cell?.getChars() || " ";
      const previous = runs.at(-1);
      if (previous?.key === key) previous.text += text;
      else runs.push({ key, style, text });
    }
    rows.push(
      `<div class="tui-row">${runs
        .map(
          (run) =>
            `<span style="${styleAttribute(run.style)}">${escapeHtml(run.text)}</span>`,
        )
        .join("")}</div>`,
    );
  }
  return rows.join("");
}

export function terminalDocument(
  terminalRows: string,
  options: TerminalPaintOptions,
): { html: string; cssWidth: number; cssHeight: number } {
  const cssWidth = Math.ceil(
    options.cols * options.fontSize * 0.62 + options.padding * 2,
  );
  const cssHeight = Math.ceil(
    options.rows * options.fontSize * options.lineHeight + options.padding * 2,
  );
  const html = `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <style>
      html, body { margin: 0; padding: 0; background: transparent; }
      body { display: inline-block; padding: 16px; }
      [data-tui-shot] {
        box-sizing: border-box;
        width: ${cssWidth}px;
        height: ${cssHeight}px;
        overflow: hidden;
        padding: ${options.padding}px;
        border: 1px solid rgba(185, 168, 255, .18);
        border-radius: ${options.borderRadius}px;
        background: ${options.background};
        color: ${options.foreground};
        box-shadow: 0 18px 44px rgba(0, 0, 0, .28);
        font-family: ${options.fontFamily};
        font-size: ${options.fontSize}px;
        font-variant-ligatures: none;
        font-weight: 400;
        line-height: ${options.lineHeight};
        text-rendering: geometricPrecision;
      }
      .tui-row {
        height: ${options.lineHeight}em;
        overflow: hidden;
        white-space: pre;
      }
    </style>
  </head>
  <body>
    <div data-tui-shot role="img" aria-label="Terminal screenshot">${terminalRows}</div>
  </body>
</html>`;
  return { html, cssWidth, cssHeight };
}
