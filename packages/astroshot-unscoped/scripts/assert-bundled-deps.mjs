#!/usr/bin/env node
/**
 * Standalone entry point for the bundle contract assertions so CI and
 * `npm test --workspace astroshot` can run them without packing.
 */
import {
  assertBundleDeclaration,
  assertBundledDependencyUnion,
} from "./bundle-contract.mjs";

assertBundleDeclaration();
assertBundledDependencyUnion();
console.log(
  "assert-bundled-deps: every bundled package is bundled at the release " +
    "version and its unscoped runtime dependencies are declared by the wrapper",
);
