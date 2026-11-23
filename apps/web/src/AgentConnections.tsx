import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, Check, Copy, ExternalLink, Terminal } from "lucide-react";
import { request } from "./api";
import { Modal } from "./ui";
import claudeLogo from "./assets/claude-spark.svg";
import chatgptLogo from "./assets/openai-blossom.svg";
import "./agent-connections.css";

export type AgentConnection = { id: string; name: string; allow_send: boolean; allow_delete: boolean; revoked: boolean; created_at: number };
type Permissions = Pick<AgentConnection, "name" | "allow_send" | "allow_delete">;

export const agentConnectionsApi = {
  list: () => request<AgentConnection[]>("/agent-connections"),
  create: (name: string) => request<{ connection: AgentConnection; token: string }>("/agent-connections", { method: "POST", body: JSON.stringify({ name }) }),
  edit: (id: string, value: Permissions) => request<AgentConnection>(`/agent-connections/${encodeURIComponent(id)}`, { method: "PUT", body: JSON.stringify(value) }),
  revoke: (id: string) => request<void>(`/agent-connections/${encodeURIComponent(id)}`, { method: "DELETE" }),
};

export function AgentConnections({ userId }: { userId: string }) {
  const queryClient = useQueryClient();
  const queryKey = ["agent-connections", userId];
  const connections = useQuery({ queryKey, queryFn: agentConnectionsApi.list });
  const [editing, setEditing] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<AgentConnection | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const revokeButton = useRef<HTMLButtonElement>(null);
  const update = (connection: AgentConnection) => queryClient.setQueryData<AgentConnection[]>(queryKey, old =>
    old?.some(c => c.id === connection.id)
      ? old.map(c => c.id === connection.id ? connection : c)
      : [connection, ...(old ?? [])]
  );
  const revoke = useMutation({
    mutationFn: agentConnectionsApi.revoke,
    onSuccess: (_, id) => {
      queryClient.setQueryData<AgentConnection[]>(queryKey, old => old?.filter(c => c.id !== id));
      setRevoking(null);
    },
  });
  return <section className="workspace-notifications agent-connections" aria-labelledby="agent-connections-heading">
    <header className="workspace-account-identity">
      <span className="workspace-account-icon"><Bot size={20} aria-hidden="true" /></span>
      <div>
        <h2 ref={heading} tabIndex={-1} id="agent-connections-heading">Agent connections</h2>
        <p>Connect an AI assistant to your mail and drafts.</p>
      </div>
    </header>
    <ClientSetup onCreate={update} />
    <div className="agent-list">
      {connections.isPending && <p role="status">Loading connections…</p>}
      {connections.isError && <p role="alert">Couldn’t load connections. <button onClick={() => void connections.refetch()}>Try again</button></p>}
      {connections.data?.length === 0 && <p>No active connections.</p>}
      {connections.data?.map(connection => <ConnectionRow
        key={connection.id}
        connection={connection}
        editing={editing === connection.id}
        onEdit={value => setEditing(value ? connection.id : null)}
        onSave={update}
        onRevoke={button => { revokeButton.current = button; revoke.reset(); setRevoking(connection); }}
      />)}
    </div>
    {revoking && <Modal title={`Revoke ${revoking.name}?`} onClose={() => { if (!revoke.isPending) setRevoking(null); }} restoreFocus={() => revokeButton.current?.isConnected ? revokeButton.current : heading.current}>
      <div className="agent-confirm">
        <p>Apps using this connection will lose access to your mail and drafts.</p>
        {revoke.isError && <p role="alert">Couldn’t revoke the connection. Try again.</p>}
        <footer>
          <button disabled={revoke.isPending} onClick={() => setRevoking(null)}>Cancel</button>
          <button className="destructive" disabled={revoke.isPending} onClick={() => revoke.mutate(revoking.id)}>{revoke.isPending ? "Revoking…" : "Revoke connection"}</button>
        </footer>
      </div>
    </Modal>}
  </section>;
}

function ConnectionRow({ connection, editing, onEdit, onSave, onRevoke }: {
  connection: AgentConnection;
  editing: boolean;
  onEdit: (editing: boolean) => void;
  onSave: (connection: AgentConnection) => void;
  onRevoke: (button: HTMLButtonElement) => void;
}) {
  const editButton = useRef<HTMLButtonElement>(null);
  const [permissions, setPermissions] = useState<Permissions>(connection);
  const finish = () => { onEdit(false); requestAnimationFrame(() => editButton.current?.focus()); };
  const save = useMutation({
    mutationFn: () => agentConnectionsApi.edit(connection.id, {
      name: permissions.name.trim(),
      allow_send: permissions.allow_send,
      allow_delete: permissions.allow_delete,
    }),
    onSuccess: result => { onSave(result); finish(); },
  });
  const allowed = [connection.allow_send && "Send mail", connection.allow_delete && "Delete mail"].filter(Boolean);
  return <article className="agent-row" aria-label={connection.name}>
    <div className="agent-row-header">
      <div>
        <h3>{connection.name}</h3>
        <small>{allowed.length ? allowed.join(" · ") : "Mail and drafts"}</small>
      </div>
      <div className="agent-actions">
        <button ref={editButton} aria-expanded={editing} aria-label={`Edit ${connection.name}`} onClick={() => {
          if (editing) { finish(); return; }
          save.reset();
          setPermissions(connection);
          onEdit(true);
        }} disabled={save.isPending}>Edit</button>
        <button aria-label={`Revoke ${connection.name}`} className="remove-account-link" onClick={event => onRevoke(event.currentTarget)}>Revoke</button>
      </div>
    </div>
    {editing && <form className="agent-inline-form" onSubmit={event => {
      event.preventDefault();
      if (!save.isPending && permissions.name.trim()) save.mutate();
    }}>
      <label>Connection name<input autoFocus required maxLength={100} autoComplete="off" value={permissions.name} onChange={event => setPermissions({ ...permissions, name: event.target.value })} disabled={save.isPending} /></label>
      <label className="agent-checkbox"><input type="checkbox" checked={permissions.allow_send} onChange={event => setPermissions({ ...permissions, allow_send: event.target.checked })} disabled={save.isPending} />Allow sending</label>
      <label className="agent-checkbox"><input type="checkbox" checked={permissions.allow_delete} onChange={event => setPermissions({ ...permissions, allow_delete: event.target.checked })} disabled={save.isPending} />Allow deleting, including permanent deletion</label>
      {save.isError && <p role="alert">Couldn’t save changes. Try again.</p>}
      <footer><button type="button" disabled={save.isPending} onClick={finish}>Cancel</button><button className="primary" disabled={save.isPending || !permissions.name.trim()}>Save changes</button></footer>
    </form>}
  </article>;
}

const clients = [
  { id: "chatgpt", label: "ChatGPT" },
  { id: "claude", label: "Claude" },
  { id: "hermes", label: "Hermes Agent" },
  { id: "other", label: "Other AI agent" },
] as const;

function ClientSetup({ onCreate }: { onCreate: (connection: AgentConnection) => void }) {
  const [client, setClient] = useState<(typeof clients)[number]["id"]>("chatgpt");
  const [creatingToken, setCreatingToken] = useState(false);
  const endpoint = new URL("/mcp", window.location.origin).href;
  const check = useMutation({ mutationFn: () => checkEndpoint(endpoint) });
  return <div className="agent-client-setup">
    <fieldset className="agent-client-options">
      <legend>Choose an assistant</legend>
      {clients.map(({ id, label }) => <label key={id}>
        <input type="radio" name="dakia-agent-client" value={id} checked={client === id} disabled={creatingToken} onChange={() => setClient(id)} />
        <span className="agent-client-option">
          {id === "chatgpt" && <img className="agent-openai-logo" src={chatgptLogo} alt="" />}
          {id === "claude" && <img src={claudeLogo} alt="" />}
          {id === "hermes" && <Terminal size={20} aria-hidden="true" />}
          {id === "other" && <Bot size={20} aria-hidden="true" />}
          {label}
        </span>
      </label>)}
    </fieldset>

    <div className="agent-server">
      <div className="agent-server-url"><span>Server URL</span><code>{endpoint}</code></div>
      <CopyButton value={endpoint} label="Copy URL" />
    </div>
    {client === "chatgpt" && <>
      <ol>
        <li>In ChatGPT Plugins, choose <strong>Add custom MCP server</strong>.</li>
        <li>Name it <strong>Dakia</strong>, paste the server URL, and choose <strong>OAuth</strong> with dynamic registration.</li>
        <li>Approve access in Dakia, then install the plugin.</li>
      </ol>
      <a className="agent-client-link" href="https://chatgpt.com/plugins" target="_blank" rel="noreferrer">Open ChatGPT <ExternalLink size={14} aria-hidden="true" /></a>
    </>}
    {client === "claude" && <>
      <ol>
        <li>In Claude, go to <strong>Customize → Connectors</strong> and choose <strong>Add custom connector</strong>.</li>
        <li>Name it <strong>Dakia</strong> and paste the server URL. If prompted, choose <strong>Register automatically</strong>.</li>
        <li>Choose <strong>Connect</strong> and approve access in Dakia.</li>
      </ol>
      <details className="agent-extra"><summary>Claude Code</summary>
        <p>Run this command, then open <code>/mcp</code> in Claude Code to sign in.</p>
        <code className="agent-command">{`claude mcp add --transport http dakia ${endpoint}`}</code>
      </details>
    </>}
    {client === "hermes" && <>
      <p>Run this command, approve access in your browser, then choose the tools to enable in Hermes.</p>
      <code className="agent-command">{`hermes mcp add dakia --url "${endpoint}" --auth oauth`}</code>
    </>}
    {client === "other" && <>
      <p>Add the server URL using <strong>Streamable HTTP</strong> and <strong>OAuth</strong> with dynamic registration. Approve access in Dakia.</p>
      <ManualAccess busy={creatingToken} onBusy={setCreatingToken} onCreate={onCreate} />
    </>}
    <div className="agent-check">
      <button type="button" onClick={() => check.mutate()} disabled={check.isPending}>Check server URL</button>
      <span role="status">{check.isPending ? "Checking…" : check.data ?? ""}</span>
    </div>
  </div>;
}

async function checkEndpoint(endpoint: string) {
  if (!endpoint.startsWith("https://")) return "Use a public HTTPS address for cloud assistants.";
  const discovery = "/.well-known/oauth-protected-resource/mcp";
  try {
    const options = { credentials: "omit", redirect: "error", cache: "no-store", signal: AbortSignal.timeout(5000) } as const;
    const [metadata, challenge] = await Promise.all([
      fetch(discovery, options),
      fetch(endpoint, {
        ...options,
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" }),
      }),
    ]);
    if (!metadata.ok || (await metadata.json()).resource !== endpoint ||
      challenge.status !== 401 || !challenge.headers.get("WWW-Authenticate")?.includes(new URL(discovery, endpoint).href)) {
      return "Server URL or OAuth discovery is misconfigured.";
    }
    return "Reachable from this browser. Cloud assistants still need public access.";
  } catch {
    return "Couldn’t reach the server URL from this browser.";
  }
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [status, setStatus] = useState("");
  return <div className="agent-copy">
    <button type="button" onClick={async () => {
      try { await navigator.clipboard.writeText(value); setStatus("Copied"); }
      catch { setStatus("Select and copy the text manually."); }
    }}>{status === "Copied" ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}{label}</button>
    <span role="status">{status}</span>
  </div>;
}

function ManualAccess({ busy, onBusy, onCreate }: {
  busy: boolean;
  onBusy: (busy: boolean) => void;
  onCreate: (connection: AgentConnection) => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [token, setToken] = useState("");
  const [error, setError] = useState(false);
  return <div className="agent-extra">
    <button type="button" aria-expanded={open} disabled={busy} onClick={() => {
      setOpen(!open); setName(""); setToken(""); setError(false);
    }}>{open ? "Close manual setup" : "Use a token instead"}</button>
    {open && (token ? <div className="agent-inline-form">
      <p>Add this header to your client. Save it before closing setup.</p>
      <code className="agent-command">{`Authorization: Bearer ${token}`}</code>
      <CopyButton value={`Bearer ${token}`} label="Copy header value" />
    </div> : <form className="agent-inline-form" onSubmit={async event => {
      event.preventDefault();
      if (busy || !name.trim()) return;
      setError(false);
      onBusy(true);
      try {
        const result = await agentConnectionsApi.create(name.trim());
        setToken(result.token);
        onCreate(result.connection);
      } catch {
        setError(true);
      } finally {
        onBusy(false);
      }
    }}>
      <label>Connection name<input autoFocus required maxLength={100} autoComplete="off" value={name} onChange={event => setName(event.target.value)} disabled={busy} /></label>
      {error && <p role="alert">Couldn’t confirm creation. Check your connections before trying again.</p>}
      <footer><button className="primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create token"}</button></footer>
    </form>)}
  </div>;
}
