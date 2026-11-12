import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { Account, ConnectAccount, DiscoveredSettings } from "@dakia/api-types";
import { request, api } from "./api";
import { Modal, ErrorNotice } from "./ui";
export function AccountSetup({
  onClose,
  account,
}: {
  onClose: () => void;
  account?: Account | null;
}) {
  const client = useQueryClient();
  const [provider, setProvider] = useState<Account["provider"]>(
    account?.provider || "imap",
  );
  const [gmailEnabled, setGmailEnabled] = useState<boolean | null>(null);
  const [microsoftEnabled, setMicrosoftEnabled] = useState<boolean | null>(null);
  const [appPassword, setAppPassword] = useState(false);
  useEffect(() => {
    let active = true;
    void request<{ enabled: boolean }>("/accounts/gmail").then((result) => {
      if (active) setGmailEnabled(result.enabled);
    }).catch(() => { if (active) setGmailEnabled(false); });
    void request<{ enabled: boolean }>("/accounts/microsoft").then((result) => {
      if (active) setMicrosoftEnabled(result.enabled);
    }).catch(() => { if (active) setMicrosoftEnabled(false); });
    return () => { active = false; };
  }, []);
  const googleSignIn = provider === "gmail" && gmailEnabled === true && !appPassword;
  const microsoftSignIn = provider === "microsoft" && microsoftEnabled === true;
  const signIn = googleSignIn || microsoftSignIn;
  const checkingSignIn = (provider === "gmail" && gmailEnabled === null) || (provider === "microsoft" && microsoftEnabled === null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [saved, setSaved] = useState<string>();
  const [existingId, setExistingId] = useState(account?.id);
  const [email, setEmail] = useState(account?.email || "");
  const [settings, setSettings] = useState<DiscoveredSettings | null>(null);
  const [manual, setManual] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [fields, setFields] = useState({ imap_host: "", imap_port: 993, imap_tls: "implicit", smtp_host: "", smtp_port: 465, smtp_tls: "implicit", username: "" });
  const generation = useRef(0);
  useEffect(() => {
    const version = ++generation.current;
    const controller = new AbortController();
    if (provider !== "imap" || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email) || manual || saved) {
      setDiscovering(false);
      return;
    }
    setDiscovering(true);
    const timer = window.setTimeout(async () => {
      try {
        const found = await api.discover(email, controller.signal);
        if (version !== generation.current) return;
        setSettings(found);
        if (found) setFields(found);
        else setManual(true);
      } catch {
        if (version === generation.current) setManual(true);
      } finally {
        if (version === generation.current) setDiscovering(false);
      }
    }, 500);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [email, provider, manual, saved]);
  function editSettings() {
    ++generation.current;
    setDiscovering(false);
    setManual(true);
  }
  async function connect(form: HTMLFormElement) {
    setBusy(true);
    setError(undefined);
    const data = new FormData(form);
    const str = (key: string) => String(data.get(key) || "");
    try {
      if (signIn) {
        const path = microsoftSignIn ? "/accounts/microsoft" : "/accounts/gmail";
        const result = await request<{ url: string }>(path, {
          method: "POST",
          body: JSON.stringify({ email: str("email"), display_name: str("display_name"), account_id: existingId || null }),
        });
        window.location.assign(result.url);
        return;
      }
      let id = saved;
      if (!id) {
        const payload: ConnectAccount = {
          email: str("email"),
          display_name: str("display_name"),
          provider: provider === "gmail" ? "gmail" : settings?.provider || "imap",
          username: str("username") || str("email"),
          password: str("password"),
          imap_host: provider === "gmail" ? "imap.gmail.com" : str("imap_host"),
          imap_port: provider === "gmail" ? 993 : Number(str("imap_port")),
          imap_tls:
            provider === "gmail"
              ? "implicit"
              : (str("imap_tls") as ConnectAccount["imap_tls"]),
          smtp_host: provider === "gmail" ? "smtp.gmail.com" : str("smtp_host"),
          smtp_port: provider === "gmail" ? 465 : Number(str("smtp_port")),
          smtp_tls:
            provider === "gmail"
              ? "implicit"
              : (str("smtp_tls") as ConnectAccount["smtp_tls"]),
        };
        const result = existingId
          ? await api.reconnect(existingId, payload)
          : await api.connect(payload);
        id = result.id;
        setExistingId(id);
        setSaved(id);
        (form.elements.namedItem("password") as HTMLInputElement).value = "";
      }
      await api.probe(id);
      await client.invalidateQueries({ queryKey: ["accounts"] });
      onClose();
    } catch (e) {
      setError(e);
      await client.invalidateQueries({ queryKey: ["accounts"] });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={account ? "Reconnect account" : "Connect an account"}
      className="account-setup"
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="setup"
        onSubmit={(e) => {
          e.preventDefault();
          void connect(e.currentTarget);
        }}
      >
        <ErrorNotice error={error} />
        <label>
          Provider
          <select title="Email provider"
            value={provider}
            disabled={busy || !!saved}
            onChange={(e) => setProvider(e.target.value as typeof provider)}
          >
            <option value="gmail">Gmail</option>
            <option value="microsoft">Microsoft (Outlook / Microsoft 365)</option>
            <option value="imap">Automatic</option>
          </select>
        </label>
        <label>
          Account name
          <input
            name="display_name"
            defaultValue={account?.display_name}
            required
            placeholder="Personal"
            disabled={busy || !!saved}
          />
        </label>
        <label>
          Email
          <input
            name="email"
            value={email}
            onChange={(e) => {
              ++generation.current;
              setEmail(e.target.value);
              setSettings(null);
              setManual(false);
              setFields({ imap_host: "", imap_port: 993, imap_tls: "implicit", smtp_host: "", smtp_port: 465, smtp_tls: "implicit", username: "" });
            }}
            type="email"
            required
            autoComplete="username"
            disabled={busy || !!saved}
          />
        </label>
        {provider === "gmail" && gmailEnabled && <label className="checkbox-label">
          <input type="checkbox" checked={appPassword} disabled={busy || !!saved} onChange={(e) => setAppPassword(e.target.checked)} />
          Use an app password
        </label>}
        {checkingSignIn && <p role="status">Checking sign-in options…</p>}
        {provider === "microsoft" && microsoftEnabled === false && <p role="status">Microsoft sign-in is not configured. Ask your Dakia operator to follow the Microsoft setup guide.</p>}
        {provider === "microsoft" && microsoftEnabled && <p>Connect Outlook.com, Hotmail, or Microsoft 365 securely with Microsoft. No mail password is stored.</p>}
        {provider !== "microsoft" && !googleSignIn && !checkingSignIn && <label>
          {provider === "gmail" ? "App password" : "Password"}
          <input
            name="password"
            type="password"
            required={!saved}
            autoComplete="new-password"
            disabled={busy || !!saved}
          />
        </label>
        }
        {provider === "imap" && (
          <>
            {discovering && <p role="status">Finding server settings…</p>}
            {settings && !manual && <p role="status">Server settings found <button title="Edit settings" type="button" disabled={busy || !!saved} onClick={editSettings}>Edit settings</button></p>}
            {!manual && !settings && !discovering && <button title="Enter server settings" type="button" disabled={busy || !!saved} onClick={editSettings}>Enter server settings</button>}
            {manual && !settings && <p>Enter your email provider’s server settings.</p>}
            <div hidden={!manual}>
            <label>
              Username
              <input name="username" autoComplete="off" value={fields.username} disabled={busy || !!saved} onChange={(e) => setFields({ ...fields, username: e.target.value })} />
            </label>
            {(["imap", "smtp"] as const).map((kind) => (
              <fieldset key={kind}>
                <legend>{kind.toUpperCase()}</legend>
                <label>
                  Host
                  <input name={`${kind}_host`} required disabled={busy || !!saved} value={fields[`${kind}_host`]} onChange={(e) => setFields({ ...fields, [`${kind}_host`]: e.target.value })} />
                </label>
                <label>
                  Port
                  <input
                    name={`${kind}_port`}
                    type="number"
                    min="1"
                    max="65535"
                    value={fields[`${kind}_port`]}
                    disabled={busy || !!saved}
                    onChange={(e) => setFields({ ...fields, [`${kind}_port`]: Number(e.target.value) })}
                    required
                  />
                </label>
                <label>
                  Encryption
                  <select title={`${kind.toUpperCase()} security`} name={`${kind}_tls`} value={fields[`${kind}_tls`]} disabled={busy || !!saved} onChange={(e) => setFields({ ...fields, [`${kind}_tls`]: e.target.value })}>
                    <option value="implicit">TLS</option>
                    <option value="starttls">Required STARTTLS</option>
                  </select>
                </label>
              </fieldset>
            ))}
            </div>
          </>
        )}
        <footer>
          {saved && (
            <button title="Edit connection"
              type="button"
              disabled={busy}
              onClick={() => setSaved(undefined)}
            >
              Edit connection
            </button>
          )}
          <button title="Connect account" className="primary" disabled={busy || checkingSignIn || (provider === "microsoft" && !microsoftEnabled) || discovering || (provider === "imap" && !settings && !manual)}>
            {busy
              ? signIn ? `Opening ${microsoftSignIn ? "Microsoft" : "Google"}…` : "Checking connection…"
              : microsoftSignIn
                ? "Continue with Microsoft"
              : googleSignIn
                ? "Continue with Google"
              : saved
                ? "Retry connection"
                : "Connect account"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
