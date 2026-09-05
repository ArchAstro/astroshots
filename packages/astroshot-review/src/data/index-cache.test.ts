import { describe, expect, it } from "vitest";

import { reconcileArrivalOrder } from "./index-cache.js";

describe("arrival order", () => {
  it("keeps known order, prepends new paths newest first, drops vanished", () => {
    const previous = ["/b", "/a", "/gone"];
    const shots = [
      { path: "/a", capturedAt: 1 },
      { path: "/b", capturedAt: 2 },
      { path: "/new-old", capturedAt: 5 },
      { path: "/new-new", capturedAt: 9 },
    ];
    expect(reconcileArrivalOrder(previous, shots)).toEqual(["/new-new", "/new-old", "/b", "/a"]);
  });
});
