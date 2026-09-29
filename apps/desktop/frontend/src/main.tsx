/**
 * The frontend entry point.
 *
 * The design tokens are imported before the components so every class resolves a
 * variable rather than falling back to a browser default.
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/tokens.css";
import "./styles/global.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("The HexaDOF root element is missing from index.html.");
}

// The theme attribute is set before React renders, so the first paint is already
// in the right theme rather than flashing the light default.
const prefersDark =
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : true;
document.documentElement.dataset.theme = prefersDark ? "dark" : "light";
document.documentElement.dataset.density = "comfortable";
document.documentElement.dataset.reducedMotion = "false";

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
