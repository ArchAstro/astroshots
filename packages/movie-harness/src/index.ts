export { encodeFrames, posterFromFrames } from "./encode.js";
export { encodeSolidPng, encodeRgbPng } from "./png.js";
export {
  assertKebabCase,
  assertSlug,
  defaultRunId,
  featureDir,
  humanize,
  resolveRoot,
} from "./paths.js";
export { MovieSession } from "./session.js";
export { finalizeManifest, sinkMovie } from "./sink.js";
export {
  formatSourceCatalog,
  recommendSource,
  SOURCE_CATALOG,
  SOURCE_DECISION_TABLE,
  sourceHintForError,
} from "./source-help.js";
export type { SourceAdvice, SourceKindHelp } from "./source-help.js";
export { recordBrowserMovie } from "./sources/browser.js";
export {
  assertDesktopToolchain,
  checkScreenRecordingAccess,
  desktopMatchFromFlags,
  ensureScreenRecordingAccess,
  formatScreenRecordingDeniedHelp,
  listDesktopWindows,
  matchDesktopWindow,
  openScreenRecordingSettings,
  recordDesktopWindowMovie,
} from "./sources/desktop-macos.js";
export type {
  DesktopWindowInfo,
  DesktopWindowMatch,
  DesktopWindowMovieOptions,
  ScreenAccessReport,
} from "./sources/desktop-macos.js";
export {
  loadFrameSession,
  markFrameSession,
  pushFrameToSession,
  startFrameSession,
  stopFrameSession,
} from "./sources/frames-store.js";
export {
  loadPtyMovieFixture,
  recordPtyMovie,
  recordTruecolorDemoMovie,
} from "./sources/pty.js";
export {
  createHeadlessTerminal,
  terminalPlainText,
  terminalToHtml,
  writeTerminal,
} from "./terminal-paint.js";
export type {
  BrowserMovieOptions,
  EncodeFramesRequest,
  ManifestStatus,
  MovieArtifact,
  MovieChapter,
  MovieFormat,
  MovieSessionOptions,
  MovieSourceKind,
  PersistedFrameSession,
  PtyAction,
  PtyKey,
  PtyMovieFixture,
  SinkMovieRequest,
  SinkMovieResult,
  Size,
} from "./types.js";
