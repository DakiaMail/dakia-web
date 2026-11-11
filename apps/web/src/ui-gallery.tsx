// Separate Vite development entry. This is not included in the production build.
import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Button, ConfirmationButton, DateTimeSelector, DakiaProvider, IconButton, DropdownMenu, DropdownMenuItem, DropdownSubmenu, DropdownMenuDivider, MailActionButton, useDakiaColorScheme } from "@dakia/ui";
import { Archive, Plus, SunMoon } from "lucide-react";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "./styles.css";
import "@dakia/ui/styles.css";

function Gallery() {
  const { colorScheme, setColorScheme } = useDakiaColorScheme();
  const [status, setStatus] = useState<"idle" | "pending" | "success" | "error">("idle");
  const [picker, setPicker] = useState<"Snooze until" | "Send later" | null>(null);
  const [chosenTime, setChosenTime] = useState("");
  const [menuResult, setMenuResult] = useState("");
  const [category, setCategory] = useState("People");
  const [loading, setLoading] = useState(false);
  const [fail, setFail] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return <main style={{ maxWidth: 720, padding: 24, margin: "0 auto" }}>
    <h1>Dakia UI components</h1>
    <IconButton label={colorScheme === "dark" ? "Light theme" : "Dark theme"} onClick={() => setColorScheme(colorScheme === "dark" ? "light" : "dark")}><SunMoon size={18} /></IconButton>
    <h2 style={{ marginTop: 24 }}>Buttons</h2>
    <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
      <Button>Primary</Button><Button variant="secondary">Secondary</Button><Button variant="tertiary">Tertiary</Button><Button tone="danger">Delete</Button><Button disabled>Unavailable</Button><Button loading>Loading</Button>
    </div>
    <h2 style={{ marginTop: 24 }}>Interaction states</h2>
    {(["primary", "secondary", "tertiary"] as const).map(variant => <div key={variant} style={{ display: "flex", gap: 12, flexWrap: "wrap", marginBlock: 12 }}>
      <Button variant={variant}>{variant} ready</Button>
      <Button variant={variant} disabled>{variant} disabled</Button>
      <Button variant={variant} loading>{variant} loading</Button>
      <IconButton variant={variant} label={`${variant} icon action`}><Plus size={16} /></IconButton>
      <IconButton variant={variant} label={`${variant} icon disabled`} disabled><Plus size={16} /></IconButton>
      <IconButton variant={variant} label={`${variant} icon loading`} loading><Plus size={16} /></IconButton>
    </div>)}
    <Button keepFocusable loading={loading} onClick={() => { setLoading(true); timer.current = setTimeout(() => setLoading(false), 1200); }}>Save changes</Button>
    <div style={{ marginBlock: 16 }}><MailActionButton label="Archive" icon={<Archive size={18} />} shortcutHint={<kbd className="key-hint" aria-hidden="true">E</kbd>} /></div>
    <h2 style={{ marginTop: 24 }}>Dropdown menus</h2>
    <DropdownMenu label="Example actions" trigger={<Button variant="secondary">Actions</Button>}>
      <DropdownMenuItem label="Archive" icon={<Archive size={16} />} shortcutHint={<kbd>E</kbd>} onClick={() => setMenuResult("Archived")} />
      <DropdownMenuItem label="Unavailable action" disabled />
      <DropdownSubmenu label="Category">
        {["People", "Notifications", "Promotions"].map(label => <DropdownMenuItem key={label} label={label} checked={category === label} onClick={() => { setCategory(label); setMenuResult(label); }} />)}
      </DropdownSubmenu>
      <DropdownMenuDivider />
      <DropdownMenuItem label="Settings" onClick={() => setMenuResult("Settings selected")} />
    </DropdownMenu>
    {menuResult && <p role="status">{menuResult}</p>}
    <h2 style={{ marginTop: 24 }}>Confirmation</h2>
    <label style={{ display: "block", marginBottom: 12 }}><input type="checkbox" checked={fail} onChange={event => setFail(event.target.checked)} />Simulate a failed save</label>
    <ConfirmationButton status={status} onClick={() => { setStatus("pending"); timer.current = setTimeout(() => setStatus(fail ? "error" : "success"), 700); }}>Save translation settings</ConfirmationButton>
    {status === "error" && <p role="alert">Couldn’t save. Try again.</p>}
    <h2 style={{ marginTop: 24 }}>Date and time</h2>
    <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
      <Button onClick={() => setPicker("Snooze until")}>Snooze</Button>
      <Button variant="secondary" onClick={() => setPicker("Send later")}>Send later</Button>
    </div>
    {chosenTime && <p role="status">Selected: {chosenTime}</p>}
    {picker && <DateTimeSelector opened title={picker} confirmLabel={picker === "Snooze until" ? "Snooze" : "Set send time"}
      value={chosenTime} onChoose={setChosenTime} onClose={() => setPicker(null)} />}
  </main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><DakiaProvider><Gallery /></DakiaProvider></StrictMode>);
