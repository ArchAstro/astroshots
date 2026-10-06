#!/usr/bin/env node
// astroshot Node helper: the only part of astroshot that must run in Node.
//
// It loads the user's own TSX (react-shot configs and fixtures, Ink fixtures)
// and serves or renders it. Everything else is Rust. Rust spawns this file and
// talks newline-delimited JSON over stdio; see `rust/astroshot/src/node_helper.rs`
// for the typed protocol and PORTING.md decision 1.
//
// Self-contained on purpose: this file does NOT import the workspace packages'
// `src` (TypeScript) or `dist` (needs a build). It re-implements the few
// Node-only pieces of `react-shot/src/{config,create-server,stubs}.ts`,
// `react-shot/src/shot.ts` (fixture meta) and `tui-shot/src/{shot,render-ink}.ts`,
// and resolves `tsx`, `vite`, `@vitejs/plugin-react` from the helper's own
// node_modules (walking up from this file), falling back to
// `packages/react-shot` and `packages/tui-shot` in a source checkout.
//
// Protocol (one JSON object per line, UTF-8):
//   stdout  <- {"ready":true,"protocol":1,"node":"v22..."}   once at startup
//   stdin   -> {"id":1,"cmd":"<command>", ...args}
//   stdout  <- {"id":1,"result":{...}}  or  {"id":1,"error":{"message","stack"}}
// stdin EOF or {"cmd":"shutdown"} closes every server and exits 0. Requests are
// processed one at a time, in order. Anything user code writes to stdout is
// diverted to stderr so it cannot corrupt the protocol.

import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import readline from "node:readline";
import { EventEmitter } from "node:events";
import { fileURLToPath, pathToFileURL } from "node:url";

const PROTOCOL = 1;
const HELPER_DIR = path.dirname(fileURLToPath(import.meta.url));

// Keep stdout for the protocol only.
const protocolWrite = process.stdout.write.bind(process.stdout);
process.stdout.write = (...args) => process.stderr.write(...args);
function send(message) {
  protocolWrite(`${JSON.stringify(message)}\n`);
}

// ---------------------------------------------------------------- resolution

const FALLBACK_BASES = [
  HELPER_DIR,
  path.resolve(HELPER_DIR, "../../packages/react-shot"),
  path.resolve(HELPER_DIR, "../../packages/tui-shot"),
].filter((dir) => fs.existsSync(dir));

function resolveFrom(bases, name) {
  for (const base of bases) {
    const resolver = createRequire(path.join(path.resolve(base), "__helper__.cjs"));
    try {
      return resolver.resolve(name);
    } catch (error) {
      if (error.code !== "MODULE_NOT_FOUND") throw error;
    }
  }
  return null;
}

async function importOwn(name) {
  try {
    return await import(name);
  } catch (error) {
    if (error.code !== "ERR_MODULE_NOT_FOUND") throw error;
    const entry = resolveFrom(FALLBACK_BASES, name);
    if (!entry) {
      throw new Error(
        `astroshot node helper could not load "${name}". Run \`npm install\` next to the helper.`,
      );
    }
    return import(pathToFileURL(entry).href);
  }
}

let tsxApi = null;
async function tsImport(file) {
  tsxApi ??= await importOwn("tsx/esm/api");
  return tsxApi.tsImport(file, import.meta.url);
}

// -------------------------------------------------------------- react config

const CONFIG_NAMES = [
  "react-shot.config.ts",
  "react-shot.config.mts",
  "react-shot.config.js",
  "react-shot.config.mjs",
];

function findConfigPath(startDir) {
  let dir = path.resolve(startDir);
  while (true) {
    for (const name of CONFIG_NAMES) {
      const candidate = path.join(dir, name);
      if (fs.existsSync(candidate)) return candidate;
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

async function loadConfig(configPath) {
  if (!configPath) return {};
  const absolutePath = path.resolve(configPath);
  if (!fs.existsSync(absolutePath)) {
    throw new Error(`react-shot config not found: ${absolutePath}`);
  }
  const module =
    absolutePath.endsWith(".ts") || absolutePath.endsWith(".mts")
      ? await tsImport(absolutePath)
      : await import(`${pathToFileURL(absolutePath).href}?t=${Date.now()}`);
  const imported = module.default ?? module;
  const config =
    imported &&
    typeof imported === "object" &&
    "default" in imported &&
    Object.keys(imported).every((key) => key === "default")
      ? imported.default
      : imported;
  const dir = path.dirname(absolutePath);
  return {
    ...config,
    root: config.root ? path.resolve(dir, config.root) : dir,
    alias: config.alias
      ? Object.fromEntries(
          Object.entries(config.alias).map(([k, v]) => [k, path.resolve(dir, v)]),
        )
      : undefined,
    styles: config.styles?.map((value) => path.resolve(dir, value)),
    postcssConfig: config.postcssConfig
      ? path.resolve(dir, config.postcssConfig)
      : undefined,
  };
}

function resolvePackageRoot(fixturePath, explicitRoot, config) {
  if (explicitRoot) return path.resolve(explicitRoot);
  if (config?.root) return config.root;
  let directory = path.dirname(path.resolve(fixturePath));
  while (true) {
    if (fs.existsSync(path.join(directory, "package.json"))) return directory;
    const parent = path.dirname(directory);
    if (parent === directory) return path.dirname(path.resolve(fixturePath));
    directory = parent;
  }
}

// ------------------------------------------------------------- react: server
// Mirrors react-shot/src/create-server.ts and stubs.ts.

const nextNavigationStub = `
export function useRouter() {
  return {
    push() {}, replace() {}, prefetch() {}, back() {},
    forward() {}, refresh() {},
  };
}
export function usePathname() { return "/"; }
export function useSearchParams() { return new URLSearchParams(); }
export function useParams() { return {}; }
export function redirect() {}
export function notFound() {}
`;
const nextLinkStub = `
import React from "react";
export default function Link({ href, children, ...rest }) {
  return React.createElement("a", { href: typeof href === "string" ? href : "#", ...rest }, children);
}
`;
const nextImageStub = `
import React from "react";
export default function Image({ src, alt, width, height, ...rest }) {
  return React.createElement("img", {
    src: typeof src === "string" ? src : "",
    alt: alt || "",
    width,
    height,
    ...rest,
  });
}
`;
const serverOnlyStub = `export {};`;
const nextDynamicStub = `
import React from "react";
export default function dynamic(loader, options) {
  const Lazy = React.lazy(loader);
  const fallback = options && options.loading
    ? React.createElement(options.loading)
    : null;
  return function DynamicComponent(props) {
    return React.createElement(
      React.Suspense,
      { fallback },
      React.createElement(Lazy, props),
    );
  };
}
`;

const HOST_HTML = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>react-shot</title>
    <style>
      html,
      body {
        margin: 0;
        padding: 0;
      }
      #root {
        min-height: 100vh;
      }
    </style>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/@react-shot/entry.tsx"></script>
  </body>
</html>
`;

const FIXTURE_ID = "virtual:react-shot-fixture";
const STYLES_ID = "virtual:react-shot-styles";
const ENTRY_ID = "virtual:react-shot-entry";
const ENTRY_URL = "/@react-shot/entry.tsx";

function resolveInstalledModule(packageRoot, moduleName) {
  return resolveFrom([path.resolve(packageRoot), ...FALLBACK_BASES], moduleName);
}

function resolveInstalledPackage(packageRoot, name) {
  const entry = resolveInstalledModule(packageRoot, name);
  if (!entry) return null;
  let directory = path.dirname(entry);
  while (true) {
    const packagePath = path.join(directory, "package.json");
    if (fs.existsSync(packagePath)) {
      const manifest = JSON.parse(fs.readFileSync(packagePath, "utf8"));
      if (manifest.name === name) return directory;
    }
    const parent = path.dirname(directory);
    if (parent === directory) return null;
    directory = parent;
  }
}

async function loadTailwindVitePlugin(packageRoot) {
  const entry = resolveInstalledModule(packageRoot, "@tailwindcss/vite");
  if (!entry) return null;
  const module = await import(pathToFileURL(entry).href);
  return (module.default ?? module)();
}

function reactShotPlugin({ fixturePath, config }) {
  const fixture = path.resolve(fixturePath);
  const styleFiles = config.styles ?? [];
  return {
    name: "react-shot",
    enforce: "pre",
    resolveId(id) {
      if (id === FIXTURE_ID) return `\0${FIXTURE_ID}`;
      if (id === STYLES_ID) return `\0${STYLES_ID}`;
      if (id === ENTRY_ID || id === ENTRY_URL || id.endsWith(ENTRY_URL)) {
        return `\0${ENTRY_ID}`;
      }
      return null;
    },
    load(id) {
      if (id === `\0${FIXTURE_ID}`) {
        return `export { default } from ${JSON.stringify(fixture.replaceAll("\\", "/"))};\n`;
      }
      if (id === `\0${STYLES_ID}`) {
        return styleFiles.length === 0
          ? "export {};\n"
          : styleFiles
              .map((file) => `import ${JSON.stringify(file.replaceAll("\\", "/"))};`)
              .join("\n");
      }
      if (id === `\0${ENTRY_ID}`) {
        return `
import React from "react";
import { createRoot } from "react-dom/client";
import fixture from ${JSON.stringify(FIXTURE_ID)};
import ${JSON.stringify(STYLES_ID)};

const width = fixture.width ?? 1280;
const height = fixture.height ?? 800;
const background = fixture.background ?? "transparent";
const win = window;

win.__REACT_SHOT_META__ = {
  width: fixture.width,
  height: fixture.height,
  background: fixture.background,
  selector: fixture.selector,
  waitFor: fixture.waitFor,
  settleMs: fixture.settleMs,
  fullPage: fixture.fullPage,
  stripOverlay: fixture.stripOverlay,
  omitBackground: fixture.omitBackground,
};

document.documentElement.style.width = width + "px";
document.documentElement.style.height = height + "px";
document.body.style.width = width + "px";
document.body.style.height = height + "px";
document.body.style.background = background;
document.body.style.overflow = "hidden";

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");
root.setAttribute("data-react-shot-root", "true");
root.style.minHeight = height + "px";
root.style.width = width + "px";
root.style.background = background;

try {
  createRoot(root).render(React.createElement(React.StrictMode, null, fixture.component));
  requestAnimationFrame(function () { win.__REACT_SHOT_READY__ = true; });
} catch (error) {
  win.__REACT_SHOT_ERROR__ = error && error.stack ? error.stack : String(error);
  win.__REACT_SHOT_READY__ = true;
  root.textContent = win.__REACT_SHOT_ERROR__;
}
`;
      }
      return null;
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (request.url !== "/" && !request.url?.startsWith("/?")) {
          next();
          return;
        }
        server
          .transformIndexHtml("/", HOST_HTML)
          .then((transformed) => {
            response.setHeader("Content-Type", "text/html");
            response.end(transformed);
          })
          .catch(next);
      });
    },
  };
}

function stubPlugin(stubs) {
  return {
    name: "react-shot-stubs",
    enforce: "pre",
    resolveId(id) {
      return id in stubs ? `\0react-shot-stub:${id}` : null;
    },
    load(id) {
      if (!id.startsWith("\0react-shot-stub:")) return null;
      return stubs[id.slice("\0react-shot-stub:".length)] ?? "export {};";
    },
  };
}

async function startShotServer({ fixturePath, packageRoot: root, config }) {
  const vite = await importOwn("vite");
  const reactPlugin = (await importOwn("@vitejs/plugin-react")).default;
  const packageRoot = path.resolve(root);
  const aliases = { ...(config.alias ?? {}) };
  if (!aliases["@"]) aliases["@"] = packageRoot;

  const stubs = {
    "next/navigation": nextNavigationStub,
    "next/link": nextLinkStub,
    "next/image": nextImageStub,
    "next/dynamic": nextDynamicStub,
    "server-only": serverOnlyStub,
  };
  for (const moduleName of config.stubModules ?? []) {
    stubs[moduleName] = serverOnlyStub;
  }

  const alias = Object.entries(aliases)
    .sort(([left], [right]) => right.length - left.length)
    .map(([find, replacement]) => ({ find, replacement }));
  const reactPackage = resolveInstalledPackage(packageRoot, "react");
  const reactDomPackage = resolveInstalledPackage(packageRoot, "react-dom");
  if (reactPackage) alias.unshift({ find: "react", replacement: reactPackage });
  if (reactDomPackage) {
    alias.unshift({ find: "react-dom", replacement: reactDomPackage });
  }

  const tailwindPlugin = await loadTailwindVitePlugin(packageRoot);
  const plugins = [
    reactShotPlugin({ fixturePath, config }),
    stubPlugin(stubs),
    reactPlugin({ jsxRuntime: "automatic" }),
  ];
  if (tailwindPlugin) plugins.unshift(tailwindPlugin);

  const allowedDirectories = new Set([
    packageRoot,
    HELPER_DIR,
    ...FALLBACK_BASES,
    path.dirname(path.resolve(fixturePath)),
    ...Object.values(aliases),
    ...(config.styles ?? []).map((file) => path.dirname(file)),
  ]);

  const server = await vite.createServer({
    configFile: false,
    root: packageRoot,
    server: {
      host: "127.0.0.1",
      port: 0,
      strictPort: false,
      fs: { allow: [...allowedDirectories] },
    },
    plugins,
    resolve: {
      alias,
      dedupe: ["react", "react-dom", "react/jsx-runtime", ...(config.dedupe ?? [])],
    },
    css:
      tailwindPlugin || !config.postcssConfig
        ? undefined
        : { postcss: config.postcssConfig },
    optimizeDeps: {
      include: ["react", "react-dom", "react/jsx-runtime", "react-dom/client"],
      noDiscovery: true,
      force: true,
    },
    logLevel: "warn",
  });

  await server.listen();
  const address = server.httpServer?.address();
  if (!address || typeof address === "string") {
    await server.close();
    throw new Error("Vite server did not bind a TCP port");
  }
  return { server, url: `http://127.0.0.1:${address.port}/` };
}

// Same plain-Node import as react-shot/src/shot.ts readFixtureMeta: TSX
// usually cannot execute here, in which case the browser reports the meta.
async function readFixtureMeta(fixturePath) {
  try {
    const url = `${pathToFileURL(path.resolve(fixturePath)).href}?reactShot=${Date.now()}`;
    const module = await import(url);
    const fixture = module.default ?? module.fixture;
    if (!fixture || typeof fixture !== "object") return {};
    const meta = {};
    for (const key of [
      "width",
      "height",
      "selector",
      "waitFor",
      "settleMs",
      "background",
      "fullPage",
      "stripOverlay",
      "omitBackground",
    ]) {
      if (fixture[key] !== undefined) meta[key] = fixture[key];
    }
    return meta;
  } catch {
    return {};
  }
}

const servers = new Map();
let nextServerId = 1;

async function cmdLoadConfig({ configPath, startDir }) {
  const resolved = configPath ?? (startDir ? findConfigPath(startDir) : null);
  return { configPath: resolved, config: await loadConfig(resolved) };
}

async function cmdReactServe({ fixture, root, configPath }) {
  const fixturePath = path.resolve(fixture);
  if (!fs.existsSync(fixturePath)) {
    throw new Error(`Fixture not found: ${fixturePath}`);
  }
  const resolvedConfig = configPath ?? findConfigPath(path.dirname(fixturePath));
  const config = await loadConfig(resolvedConfig);
  const packageRoot = resolvePackageRoot(fixturePath, root, config);
  const nodeMeta = await readFixtureMeta(fixturePath);
  const { server, url } = await startShotServer({ fixturePath, packageRoot, config });
  const serverId = nextServerId++;
  servers.set(serverId, server);
  return {
    serverId,
    url,
    fixturePath,
    packageRoot,
    configPath: resolvedConfig,
    config,
    nodeMeta,
  };
}

async function cmdReactStop({ serverId }) {
  const server = servers.get(serverId);
  if (!server) return { stopped: false };
  servers.delete(serverId);
  await server.close();
  return { stopped: true };
}

// ---------------------------------------------------------------- ink render
// Mirrors tui-shot/src/shot.ts loadFixture + render-ink.ts renderInkFrame.

const CI_KEYS = [
  "CI",
  "CONTINUOUS_INTEGRATION",
  "GITHUB_ACTIONS",
  "BUILD_NUMBER",
  "RUN_ID",
  "GITLAB_CI",
  "CIRCLECI",
  "TRAVIS",
  "BUILDKITE",
];

function fakeStdin() {
  const stream = new EventEmitter();
  stream.isTTY = true;
  stream.setRawMode = () => stream;
  stream.setEncoding = () => stream;
  stream.resume = () => stream;
  stream.pause = () => stream;
  stream.ref = () => stream;
  stream.unref = () => stream;
  stream.read = () => null;
  return stream;
}

function printableText(chunk) {
  return chunk
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b[@-_]/g, "");
}

function renderInkFrame(fixture, cols, rows, runtime) {
  const chunks = [];
  const stdout = {
    columns: cols,
    rows,
    isTTY: true,
    write: (chunk) => {
      chunks.push(String(chunk));
      return true;
    },
    on() {},
    off() {},
    once() {},
    removeListener() {},
    end() {},
  };
  const saved = new Map();
  const chalkLevel = runtime.chalk.level;
  for (const key of CI_KEYS) {
    saved.set(key, process.env[key]);
    delete process.env[key];
  }
  let frame = "";
  try {
    runtime.chalk.level = 3;
    const instance = runtime.render(fixture.component, {
      debug: true,
      exitOnCtrlC: false,
      patchConsole: false,
      stdin: fakeStdin(),
      stdout,
    });
    for (let index = chunks.length - 1; index >= 0; index--) {
      if (printableText(chunks[index]).trim().length > 0) {
        frame = chunks[index];
        break;
      }
    }
    instance.unmount();
    instance.cleanup();
  } finally {
    runtime.chalk.level = chalkLevel;
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
  if (!frame) {
    throw new Error(
      "Ink produced no printable frame. Check that the fixture renders visible content.",
    );
  }
  return frame;
}

function validPositive(value, name, { integer, maximum }) {
  if (
    !Number.isFinite(value) ||
    value <= 0 ||
    value > maximum ||
    (integer && !Number.isInteger(value))
  ) {
    throw new Error(
      `${name} must be a positive${integer ? " integer" : ""} no greater than ${maximum}`,
    );
  }
  return value;
}

async function loadInkFixture(fixturePath) {
  const absolute = path.resolve(fixturePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`Fixture not found: ${absolute}`);
  }
  const fixtureRequire = createRequire(absolute);
  let inkEntry;
  let reactEntry;
  try {
    inkEntry = fixtureRequire.resolve("ink");
    reactEntry = fixtureRequire.resolve("react");
  } catch (error) {
    throw new Error(
      `Could not resolve Ink and React next to fixture ${absolute}. Install ink@7 and react@19 in the fixture project.`,
      { cause: error },
    );
  }
  const inkRequire = createRequire(inkEntry);
  const [module, inkModule, reactModule, chalkModule] = await Promise.all([
    tsImport(absolute),
    import(pathToFileURL(inkEntry).href),
    import(pathToFileURL(reactEntry).href),
    import(pathToFileURL(inkRequire.resolve("chalk")).href),
  ]);
  const fixture = module.default ?? module.fixture;
  if (!fixture?.component) {
    throw new Error(
      `Fixture ${absolute} must default-export a TuiShotFixture with a component.`,
    );
  }
  return {
    fixture,
    ink: { render: inkModule.render, chalk: chalkModule.default },
    react: reactModule.default ?? reactModule,
  };
}

async function cmdInkRender({ fixture: fixturePath, cols, rows }) {
  const context = await loadInkFixture(fixturePath);
  const { fixture } = context;
  const resolvedCols = validPositive(cols ?? fixture.cols ?? 100, "cols", {
    integer: true,
    maximum: 1_000,
  });
  const resolvedRows = validPositive(rows ?? fixture.rows ?? 30, "rows", {
    integer: true,
    maximum: 1_000,
  });
  const previousReact = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", {
    configurable: true,
    value: context.react,
    writable: true,
  });
  let ansi;
  try {
    ansi = renderInkFrame(fixture, resolvedCols, resolvedRows, context.ink);
  } finally {
    if (previousReact) Object.defineProperty(globalThis, "React", previousReact);
    else Reflect.deleteProperty(globalThis, "React");
  }
  // Raw fixture styling; Rust applies defaults and validation (shot.ts
  // renderTuiShot) and checks expectText against the stripped frame.
  const result = { ansi, cols: resolvedCols, rows: resolvedRows };
  for (const key of [
    "expectText",
    "background",
    "foreground",
    "fontFamily",
    "fontSize",
    "lineHeight",
    "padding",
    "borderRadius",
    "scale",
  ]) {
    if (fixture[key] !== undefined) result[key] = fixture[key];
  }
  return result;
}

// -------------------------------------------------------------- browser-script

// Run a user's browser movie script (`export default async (page) => ...`, or
// `run`) against the Rust-launched Chrome. `playwright-core` attaches over CDP
// and the script gets the existing page, identified by CDP target id or URL.
// Mirrors `movie-harness/src/sources/browser.ts` `runScript`.
async function cmdBrowserScript({ wsEndpoint, targetId, url, scriptPath }) {
  if (typeof wsEndpoint !== "string" || !wsEndpoint) {
    throw new Error("browser-script: wsEndpoint is required");
  }
  const absolute = path.resolve(String(scriptPath ?? ""));
  if (!fs.existsSync(absolute)) {
    throw new Error(`browser script not found: ${absolute}`);
  }
  const mod = await import(pathToFileURL(absolute).href);
  const runner = mod.default ?? mod.run;
  if (typeof runner !== "function") {
    throw new Error(
      `browser script must export default or run async function(page): ${absolute}`,
    );
  }
  const playwright = await importOwn("playwright-core");
  const chromium = playwright.chromium ?? playwright.default?.chromium;
  const browser = await chromium.connectOverCDP(wsEndpoint);
  try {
    const pages = browser.contexts().flatMap((context) => context.pages());
    let page;
    if (targetId) {
      for (const candidate of pages) {
        const session = await candidate.context().newCDPSession(candidate);
        try {
          const { targetInfo } = await session.send("Target.getTargetInfo");
          if (targetInfo.targetId === targetId) page = candidate;
        } finally {
          await session.detach().catch(() => undefined);
        }
        if (page) break;
      }
    }
    page ??= url ? pages.find((candidate) => candidate.url() === url) : undefined;
    if (!page) {
      throw new Error(
        `browser-script: could not find the page to drive (${targetId ?? url ?? "no target"})`,
      );
    }
    await runner(page);
  } finally {
    // Detaches from a CDP-attached browser; Chrome stays up for Rust.
    await browser.close().catch(() => undefined);
  }
  return { ok: true };
}

// ------------------------------------------------------------------ dispatch

const COMMANDS = {
  ping: async () => ({ protocol: PROTOCOL, node: process.version }),
  "load-config": cmdLoadConfig,
  "react-serve": cmdReactServe,
  "react-stop": cmdReactStop,
  "ink-render": cmdInkRender,
  "browser-script": cmdBrowserScript,
};

async function closeAll() {
  const open = [...servers.values()];
  servers.clear();
  await Promise.allSettled(open.map((server) => server.close()));
}

function errorPayload(error) {
  return {
    message: error instanceof Error ? error.message : String(error),
    ...(error instanceof Error && error.stack ? { stack: error.stack } : {}),
  };
}

async function handle(line) {
  let request;
  try {
    request = JSON.parse(line);
  } catch (error) {
    send({ id: null, error: { message: `invalid JSON request: ${error.message}` } });
    return true;
  }
  const id = request.id ?? null;
  if (request.cmd === "shutdown") {
    await closeAll();
    send({ id, result: {} });
    return false;
  }
  const command = COMMANDS[request.cmd];
  if (!command) {
    send({ id, error: { message: `unknown command: ${request.cmd}` } });
    return true;
  }
  try {
    send({ id, result: await command(request) });
  } catch (error) {
    send({ id, error: errorPayload(error) });
  }
  return true;
}

const lines = readline.createInterface({ input: process.stdin });
let queue = Promise.resolve();
let running = true;
lines.on("line", (line) => {
  if (!line.trim()) return;
  queue = queue.then(async () => {
    if (!running) return;
    running = await handle(line);
    if (!running) process.exit(0);
  });
});
lines.on("close", () => {
  queue = queue.then(async () => {
    await closeAll();
    process.exit(0);
  });
});
process.on("unhandledRejection", (error) => {
  process.stderr.write(`astroshot node helper: unhandled rejection: ${error?.stack ?? error}\n`);
});

send({ ready: true, protocol: PROTOCOL, node: process.version });
