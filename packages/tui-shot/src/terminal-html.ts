import xtermHeadless, {
  type IBufferCell,
  type Terminal as TerminalType,
} from "@xterm/headless";

const { Terminal } = xtermHeadless as unknown as {
  Terminal: typeof TerminalType;
};

const ANSI_16 = [
  "#2b2f3a", "#f0727a", "#54e0a0", "#f0b86e",
  "#7aa2f7", "#b9a8ff", "#5bd6e0", "#e8e8f2",
  "#5a5a7a", "#ff8d94", "#78efb6", "#ffd08a",
  "#9bbcff", "#d1c5ff", "#86eaf0", "#ffffff",
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

export interface TerminalHtmlOptions {
  cols: number;
  rows: number;
  foreground: string;
  background: string;
}

/** Interpret a real ANSI frame and return styled HTML rows from xterm's cells. */
export async function ansiFrameToHtml(
  ansi: string,
  options: TerminalHtmlOptions,
): Promise<string> {
  const terminal = new Terminal({
    cols: options.cols,
    rows: options.rows,
    allowProposedApi: true,
    convertEol: true,
    scrollback: 0,
  });
  try {
    await new Promise<void>((resolve) =>
      terminal.write(`\x1b[?7l${ansi}`, resolve),
    );

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
  } finally {
    terminal.dispose();
  }
}
