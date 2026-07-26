export const nextNavigationStub = `
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

export const nextLinkStub = `
import React from "react";
export default function Link({ href, children, ...rest }) {
  return React.createElement("a", { href: typeof href === "string" ? href : "#", ...rest }, children);
}
`;

export const nextImageStub = `
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

export const serverOnlyStub = `export {};`;

export const nextDynamicStub = `
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
