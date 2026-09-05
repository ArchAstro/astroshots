import os from "node:os";

import { Box, Text } from "ink";

import { indexCachePath } from "../data/index-cache.js";
import { abbreviateHome } from "../data/paths.js";
import { MetaRow, Rule, SectionLabel } from "./chrome.js";
import { useServices } from "./context.js";
import { theme } from "./theme.js";

export function SettingsPane({ roots, width, height }: { roots: string[]; width: number; height: number }) {
  const { capabilities, ffmpeg, rootsSource, version } = useServices();
  const inner = width - 2;
  const home = os.homedir();
  const sourceLabel =
    rootsSource === "app" ? "from the Astroshots app preferences" : rootsSource === "cli" ? "from --root" : "current directory";
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} paddingX={1} overflow="hidden">
      <Box flexShrink={0} height={1} justifyContent="space-between" width={inner}>
        <Text color={theme.blue}>‹ Stream</Text>
        <Text color={theme.muted}>astroshot review {version}</Text>
      </Box>
      <SectionLabel>WATCHED FOLDERS</SectionLabel>
      <Text color={theme.muted} wrap="wrap">
        {roots.length === 0
          ? "Pick the directories that contain your projects. Nothing is watched until you choose at least one."
          : `Every worktree below any of these folders streams into one feed (${sourceLabel}).`}
      </Text>
      {roots.length === 0 ? (
        <Text color={theme.faint}>No folders yet · astroshot review --root ~/projects</Text>
      ) : (
        roots.map((root) => (
          <Text key={root} wrap="truncate">
            <Text color={theme.green}>● </Text>
            {abbreviateHome(root, home)}
          </Text>
        ))
      )}
      <Rule width={inner} />
      <SectionLabel>GRAPHICS</SectionLabel>
      <MetaRow label="Protocol" value={capabilities.graphics === "kitty" ? "Kitty graphics" : `none · ${capabilities.reason ?? "unsupported"}`} width={inner} />
      <MetaRow label="Cell" value={`${capabilities.cellWidth}×${capabilities.cellHeight} px (${capabilities.cellSource})`} width={inner} />
      <MetaRow label="Transport" value={capabilities.fileMedium ? "file path (local)" : "inline bytes"} width={inner} />
      <MetaRow label="Session" value={[capabilities.insideSsh ? "ssh" : "local", capabilities.insideTmux ? "tmux" : null].filter(Boolean).join(" · ")} width={inner} />
      <Rule width={inner} />
      <SectionLabel>MOVIES</SectionLabel>
      <MetaRow label="ffmpeg" value={ffmpeg.ffmpeg ? `${ffmpeg.ffmpeg}${ffmpeg.version ? ` (${ffmpeg.version})` : ""}` : "not found · brew install ffmpeg"} width={inner} />
      <Text color={theme.muted} wrap="wrap">
        Movies decode through ffmpeg into frames drawn with the graphics protocol. Without ffmpeg, O opens the file in your default player.
      </Text>
      <Rule width={inner} />
      <SectionLabel>HARNESS LAYOUT</SectionLabel>
      <Text color={theme.muted}>Write frames here so Astroshots can find them:</Text>
      <Text color={theme.muted}>{"  .astroshot/<feature>/"}</Text>
      <Text color={theme.muted}>{"    manifest.json"}</Text>
      <Text color={theme.muted}>{"    0001-slug.png"}</Text>
      <Rule width={inner} />
      <MetaRow label="Index" value={abbreviateHome(indexCachePath(), home)} width={inner} />
    </Box>
  );
}
