import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { closeSharedBrowser, takeShot } from "./shot.js";

describe("react-shot programmatic concurrency", () => {
  it("serializes concurrent captures through one shared browser lifecycle", async () => {
    const directory = fs.mkdtempSync(
      path.join(os.tmpdir(), "react-shot-concurrency-"),
    );
    let activeBrowserRequests = 0;
    let maximumActiveBrowserRequests = 0;
    const readinessServer = http.createServer((request, response) => {
      const fromBrowser = /Chrome/i.test(request.headers["user-agent"] ?? "");
      if (fromBrowser) {
        activeBrowserRequests += 1;
        maximumActiveBrowserRequests = Math.max(
          maximumActiveBrowserRequests,
          activeBrowserRequests,
        );
      }
      setTimeout(() => {
        if (fromBrowser) activeBrowserRequests -= 1;
        response.setHeader("Access-Control-Allow-Origin", "*");
        response.end("ready");
      }, 500);
    });
    await new Promise<void>((resolve) =>
      readinessServer.listen(0, "127.0.0.1", resolve),
    );
    const address = readinessServer.address();
    if (!address || typeof address === "string") {
      throw new Error("readiness server did not bind a TCP port");
    }
    const fixturePath = path.join(directory, "concurrent.tsx");
    fs.writeFileSync(
      fixturePath,
      `import React, { useEffect, useState } from "react";

const readiness = fetch("http://127.0.0.1:${address.port}/ready");

function ConcurrentFixture() {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    void readiness.then(() => setReady(true));
  }, []);
  return <div data-concurrent-shot>{ready ? "Concurrent ready" : "Loading"}</div>;
}

export default {
  width: 320,
  height: 120,
  selector: "[data-concurrent-shot]",
  waitFor: "text=Concurrent ready",
  component: <ConcurrentFixture />,
};
`,
    );
    const outputs = [
      path.join(directory, "first.png"),
      path.join(directory, "second.png"),
    ];

    try {
      const written = await Promise.all(
        outputs.map((outPath) =>
          takeShot({
            fixturePath,
            outPath,
            root: path.resolve("."),
          }),
        ),
      );

      expect(written).toEqual(outputs);
      expect(maximumActiveBrowserRequests).toBe(1);
      for (const output of outputs) {
        const png = fs.readFileSync(output);
        expect(png.subarray(0, 8)).toEqual(
          Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
        );
      }
    } finally {
      await closeSharedBrowser();
      await new Promise<void>((resolve, reject) =>
        readinessServer.close((error) => (error ? reject(error) : resolve())),
      );
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});
