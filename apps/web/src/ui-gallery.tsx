// Separate Vite development entry. This is not included in the production build.
import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Button, ConfirmationButton, DakiaProvider, IconButton, useDakiaColorScheme } from "@dakia/ui";
import { SunMoon } from "lucide-react";
import "@fontsource/inter/400.css";
import "@fontsource/inter/600.css";
import "./styles.css";
import "@dakia/ui/styles.css";

function Gallery() {
  const { colorScheme, setColorScheme } = useDakiaColorScheme();
  const [status, setStatus] = useState<"idle" | "pending" | "success" | "error">("idle");
  const [fail, setFail] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return <main style={{ maxWidth: 720, padding: 24, margin: "0 auto" }}>
    <h1>Dakia UI components</h1>
    <IconButton label={colorScheme === "dark" ? "Light theme" : "Dark theme"} onClick={() => setColorScheme(colorScheme === "dark" ? "light" : "dark")}><SunMoon size={18} /></IconButton>
    <h2 style={{ marginTop: 24 }}>Buttons</h2>
    <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
      <Button>Primary</Button><Button variant="secondary">Secondary</Button><Button variant="subtle">Subtle</Button><Button variant="danger">Delete</Button><Button disabled>Unavailable</Button><Button loading>Loading</Button>
    </div>
    <h2 style={{ marginTop: 24 }}>Confirmation</h2>
    <label style={{ display: "block", marginBottom: 12 }}><input type="checkbox" checked={fail} onChange={event => setFail(event.target.checked)} />Simulate a failed save</label>
    <ConfirmationButton status={status} onClick={() => { setStatus("pending"); timer.current = setTimeout(() => setStatus(fail ? "error" : "success"), 700); }}>Save translation settings</ConfirmationButton>
    {status === "error" && <p role="alert">Couldn’t save. Try again.</p>}
  </main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><DakiaProvider><Gallery /></DakiaProvider></StrictMode>);
