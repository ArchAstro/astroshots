/** Desktop integrations: reveal, open, and copy — best effort per platform. */
import { execFile } from "node:child_process";
import path from "node:path";

function run(command: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile(command, args, (error) => (error ? reject(error) : resolve()));
  });
}

export async function revealInFileManager(target: string): Promise<void> {
  if (process.platform === "darwin") return run("open", ["-R", target]);
  if (process.platform === "win32") return run("explorer", [`/select,${target}`]);
  return run("xdg-open", [path.dirname(target)]);
}

export async function openWithDefaultApp(target: string): Promise<void> {
  if (process.platform === "darwin") return run("open", [target]);
  if (process.platform === "win32") return run("cmd", ["/c", "start", "", target]);
  return run("xdg-open", [target]);
}

/** Put the image itself (not its path) on the clipboard, like the app's Copy Image. */
export async function copyImageToClipboard(imagePath: string): Promise<void> {
  if (process.platform === "darwin") {
    const escaped = imagePath.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    const kind = /\.png$/i.test(imagePath) ? "«class PNGf»" : "JPEG picture";
    return run("osascript", ["-e", `set the clipboard to (read (POSIX file "${escaped}") as ${kind})`]);
  }
  if (process.platform === "linux") {
    return run("sh", ["-c", `xclip -selection clipboard -t image/png -i "${imagePath.replace(/"/g, '\\"')}"`]);
  }
  throw new Error("Copy image is not supported on this platform");
}
