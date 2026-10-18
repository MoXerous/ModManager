import React from "react";
import ReactDOM from "react-dom/client";

// Latin + latin-ext for the UI text, arabic for the Arabic face, latin for the
// mono chip and version strings. Weights 400/500/600 only (board CSS uses 600
// and 500; the rest would ship pixels nobody renders).
import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-500.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-ext-400.css";
import "@fontsource/ibm-plex-sans/latin-ext-500.css";
import "@fontsource/ibm-plex-sans/latin-ext-600.css";
import "@fontsource/ibm-plex-sans-arabic/arabic-400.css";
import "@fontsource/ibm-plex-sans-arabic/arabic-500.css";
import "@fontsource/ibm-plex-sans-arabic/arabic-600.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";

import "./styles/theme.css";
import "./app/i18n";

import App from "./App";
import { applyToDocument, getPreferences } from "./app/preferences";

// A module script runs before the first paint, so this settles theme, lang and
// direction without a flash.
applyToDocument(getPreferences());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);