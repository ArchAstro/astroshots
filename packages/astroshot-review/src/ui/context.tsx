import { createContext, useContext } from "react";

import type { ReviewStore } from "../data/store.js";
import type { ImageService } from "../images/service.js";
import type { ImageLayer } from "../terminal/image-layer.js";
import type { TerminalCapabilities } from "../terminal/probe.js";
import type { FfmpegInfo } from "../video/ffmpeg.js";

export interface AppServices {
  store: ReviewStore;
  layer: ImageLayer;
  service: ImageService;
  capabilities: TerminalCapabilities;
  ffmpeg: FfmpegInfo;
  rootsSource: "app" | "cli" | "cwd";
  version: string;
}

export const ServicesContext = createContext<AppServices | null>(null);

export function useServices(): AppServices {
  const services = useContext(ServicesContext);
  if (!services) throw new Error("ServicesContext is missing");
  return services;
}
