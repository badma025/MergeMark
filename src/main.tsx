import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "@/components/theme-provider";
import { ErrorBoundary } from "@/components/common/ErrorBoundary";
import App from "./App";
import "./index.css";
import "katex/dist/katex.min.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary fallbackTitle="MergeMark Application Error">
      <ThemeProvider defaultTheme="dark" storageKey="mergemark-ui-theme">
        <App />
      </ThemeProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
