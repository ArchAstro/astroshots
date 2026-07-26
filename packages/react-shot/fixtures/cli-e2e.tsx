import type { ReactShotFixture } from "../src/types.js";

function ReleaseCard() {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        display: "grid",
        placeItems: "center",
        background: "rgba(0, 0, 0, 0.65)",
      }}
    >
      <article
        role="dialog"
        aria-label="Release summary"
        style={{
          boxSizing: "border-box",
          width: 400,
          padding: 20,
          borderRadius: 12,
          background: "#f8fafc",
          color: "#0f172a",
          fontFamily: "system-ui, sans-serif",
        }}
      >
        <h1 style={{ margin: "0 0 8px", fontSize: 24 }}>Ready to publish</h1>
        <p style={{ margin: 0, fontSize: 16 }}>
          A real TSX fixture rendered this PNG in Chromium.
        </p>
      </article>
    </div>
  );
}

export default {
  width: 800,
  height: 600,
  selector: "[role=dialog]",
  settleMs: 0,
  component: <ReleaseCard />,
} satisfies ReactShotFixture;
