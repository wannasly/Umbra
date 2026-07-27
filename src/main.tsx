import "@fontsource-variable/inter";
import "@fontsource-variable/manrope";
import "@fontsource-variable/jetbrains-mono";
import { polyfillCountryFlagEmojis } from "country-flag-emoji-polyfill";
import flagFontUrl from "../node_modules/country-flag-emoji-polyfill/dist/TwemojiCountryFlags.woff2?url";
import "./styles/theme.css";
import "./i18n";

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initApp } from "./lib/events";

polyfillCountryFlagEmojis(undefined, flagFontUrl);
initApp();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
