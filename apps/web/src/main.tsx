import { deviceTranslator } from "./deviceTranslation";
import { localMailCache } from "./localMailCache";
import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "./styles.css";
import "@dakia/ui/styles.css";
import { DakiaProvider } from "@dakia/ui";
import { App } from "./App";
import { InstallProvider } from "./InstallApp";
import { AppUpdateProvider } from "./AppUpdate";
window.addEventListener("pagehide", () => deviceTranslator.reset());
const client = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 15000, refetchOnWindowFocus: true },
  },
});
window.addEventListener("dakia-auth-expired", event => {
  deviceTranslator.reset();
  void localMailCache.clear();
  if ((event as CustomEvent).detail !== "/session") void client.invalidateQueries({ queryKey: ["session"] });
});
async function start() {
  if (import.meta.env.MODE === "demo") {
    const { installDemo } = await import("./demo/bootstrap");
    await installDemo();
  }
  createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <DakiaProvider>
        <QueryClientProvider client={client}>
          <InstallProvider><AppUpdateProvider><App /></AppUpdateProvider></InstallProvider>
        </QueryClientProvider>
      </DakiaProvider>
    </React.StrictMode>,
  );
}
void start();
