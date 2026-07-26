import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { tsImport } from "tsx/esm/api";
import type { ReactShotConfig } from "./types.js";

const CONFIG_NAMES = [
  "react-shot.config.ts",
  "react-shot.config.mts",
  "react-shot.config.js",
  "react-shot.config.mjs",
];

export function findConfigPath(startDir: string): string | null {
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

export async function loadConfig(
  configPath: string | null | undefined,
): Promise<ReactShotConfig> {
  if (!configPath) return {};

  const absolutePath = path.resolve(configPath);
  if (!fs.existsSync(absolutePath)) {
    throw new Error(`react-shot config not found: ${absolutePath}`);
  }

  const module = absolutePath.endsWith(".ts") || absolutePath.endsWith(".mts")
    ? await tsImport(absolutePath, import.meta.url)
    : await import(`${pathToFileURL(absolutePath).href}?t=${Date.now()}`);
  const imported = (module.default ?? module) as
    | ReactShotConfig
    | { default: ReactShotConfig };
  // tsx exposes a TypeScript default export through one additional interop
  // layer when the importing package is ESM.
  const config: ReactShotConfig =
    "default" in imported &&
    Object.keys(imported).every((key) => key === "default")
      ? imported.default
      : (imported as ReactShotConfig);
  const configDirectory = path.dirname(absolutePath);

  return {
    ...config,
    root: config.root
      ? path.resolve(configDirectory, config.root)
      : configDirectory,
    alias: config.alias
      ? Object.fromEntries(
          Object.entries(config.alias).map(([key, value]) => [
            key,
            path.resolve(configDirectory, value),
          ]),
        )
      : undefined,
    styles: config.styles?.map((value) =>
      path.resolve(configDirectory, value),
    ),
    postcssConfig: config.postcssConfig
      ? path.resolve(configDirectory, config.postcssConfig)
      : undefined,
  };
}

export function resolvePackageRoot(
  fixturePath: string,
  explicitRoot?: string,
  config?: ReactShotConfig,
): string {
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
