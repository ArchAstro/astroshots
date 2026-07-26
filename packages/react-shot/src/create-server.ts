import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import react from "@vitejs/plugin-react";
import {
  createServer as createViteServer,
  type Plugin,
  type PluginOption,
  type ViteDevServer,
} from "vite";
import type { ReactShotConfig } from "./types.js";
import {
  nextDynamicStub,
  nextImageStub,
  nextLinkStub,
  nextNavigationStub,
  serverOnlyStub,
} from "./stubs.js";

const TOOL_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const HOST_DIR = path.join(TOOL_ROOT, "host");
const FIXTURE_ID = "virtual:react-shot-fixture";
const STYLES_ID = "virtual:react-shot-styles";
const ENTRY_ID = "virtual:react-shot-entry";
const ENTRY_URL = "/@react-shot/entry.tsx";

export function resolveInstalledPackage(
  packageRoot: string,
  name: string,
): string | null {
  const entry = resolveInstalledModule(packageRoot, name);
  if (!entry) return null;

  let directory = path.dirname(entry);
  while (true) {
    const packagePath = path.join(directory, "package.json");
    if (fs.existsSync(packagePath)) {
      const manifest = JSON.parse(fs.readFileSync(packagePath, "utf8")) as {
        name?: string;
      };
      if (manifest.name === name) return directory;
    }
    const parent = path.dirname(directory);
    if (parent === directory) return null;
    directory = parent;
  }
}

export function resolveInstalledModule(
  packageRoot: string,
  moduleName: string,
): string | null {
  for (const base of [path.resolve(packageRoot), TOOL_ROOT]) {
    const resolver = createRequire(path.join(base, "__react_shot_resolver__.cjs"));
    try {
      return resolver.resolve(moduleName);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "MODULE_NOT_FOUND") {
        throw error;
      }
    }
  }
  return null;
}

async function loadTailwindVitePlugin(
  packageRoot: string,
): Promise<PluginOption | null> {
  const entry = resolveInstalledModule(packageRoot, "@tailwindcss/vite");
  if (!entry) return null;

  const module = await import(pathToFileURL(entry).href);
  const tailwind = (module.default ?? module) as () => PluginOption;
  return tailwind();
}

function reactShotPlugin(options: {
  fixturePath: string;
  config: ReactShotConfig;
}): Plugin {
  const fixturePath = path.resolve(options.fixturePath);
  const styleFiles = options.config.styles ?? [];

  return {
    name: "react-shot",
    enforce: "pre",
    resolveId(id) {
      if (id === FIXTURE_ID) return `\0${FIXTURE_ID}`;
      if (id === STYLES_ID) return `\0${STYLES_ID}`;
      if (
        id === ENTRY_ID ||
        id === ENTRY_URL ||
        id.endsWith(ENTRY_URL)
      ) {
        return `\0${ENTRY_ID}`;
      }
      return null;
    },
    load(id) {
      if (id === `\0${FIXTURE_ID}`) {
        return `export { default } from ${JSON.stringify(fixturePath.replaceAll("\\", "/"))};\n`;
      }
      if (id === `\0${STYLES_ID}`) {
        return styleFiles.length === 0
          ? "export {};\n"
          : styleFiles
              .map((file) =>
                `import ${JSON.stringify(file.replaceAll("\\", "/"))};`
              )
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
        const html = fs.readFileSync(path.join(HOST_DIR, "index.html"), "utf8");
        server
          .transformIndexHtml("/", html)
          .then((transformed) => {
            response.setHeader("Content-Type", "text/html");
            response.end(transformed);
          })
          .catch(next);
      });
    },
  };
}

function stubPlugin(stubs: Record<string, string>): Plugin {
  return {
    name: "react-shot-stubs",
    enforce: "pre",
    resolveId(id) {
      return id in stubs ? `\0react-shot-stub:${id}` : null;
    },
    load(id) {
      if (!id.startsWith("\0react-shot-stub:")) return null;
      const name = id.slice("\0react-shot-stub:".length);
      return stubs[name] ?? "export {};";
    },
  };
}

export async function startShotServer(options: {
  fixturePath: string;
  packageRoot: string;
  config: ReactShotConfig;
}): Promise<{ server: ViteDevServer; url: string }> {
  const packageRoot = path.resolve(options.packageRoot);
  const aliases = { ...(options.config.alias ?? {}) };
  if (!aliases["@"]) aliases["@"] = packageRoot;

  const stubs: Record<string, string> = {
    "next/navigation": nextNavigationStub,
    "next/link": nextLinkStub,
    "next/image": nextImageStub,
    "next/dynamic": nextDynamicStub,
    "server-only": serverOnlyStub,
  };
  for (const moduleName of options.config.stubModules ?? []) {
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
  const plugins: PluginOption[] = [
    reactShotPlugin({
      fixturePath: options.fixturePath,
      config: options.config,
    }),
    stubPlugin(stubs),
    react({ jsxRuntime: "automatic" }),
  ];
  if (tailwindPlugin) plugins.unshift(tailwindPlugin);

  const allowedDirectories = new Set([
    packageRoot,
    TOOL_ROOT,
    path.dirname(path.resolve(options.fixturePath)),
    ...Object.values(aliases),
    ...(options.config.styles ?? []).map(path.dirname),
  ]);

  const server = await createViteServer({
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
      dedupe: [
        "react",
        "react-dom",
        "react/jsx-runtime",
        ...(options.config.dedupe ?? []),
      ],
    },
    css:
      tailwindPlugin || !options.config.postcssConfig
        ? undefined
        : { postcss: options.config.postcssConfig },
    optimizeDeps: {
      include: [
        "react",
        "react-dom",
        "react/jsx-runtime",
        "react-dom/client",
      ],
      // Fixtures are executable entry points, not an application tree for Vite
      // to crawl. Discovery can otherwise walk unrelated HTML reachable from
      // unusual host filesystems; declared imports still load on demand.
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
