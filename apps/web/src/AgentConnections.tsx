import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, Plus } from "lucide-react";
import { request } from "./api";
import { Modal } from "./ui";
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
  const client = useQueryClient();
  const key = ["agent-connections", userId];
  const connections = useQuery({ queryKey: key, queryFn: agentConnectionsApi.list });
  const [dialog, setDialog] = useState<"create" | "edit" | "revoke" | "setup" | null>(null);
  const [selected, setSelected] = useState<AgentConnection | null>(null);
  const [name, setName] = useState("");
  const [send, setSend] = useState(false);
  const [remove, setRemove] = useState(false);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  const close = () => { if (!busy) { setDialog(null); setToken(""); setError(""); } };
  const update = (connection: AgentConnection) => client.setQueryData<AgentConnection[]>(key, old => [...(old || []).filter(c => c.id !== connection.id), connection]);
  const open = (kind: typeof dialog, c: AgentConnection | null = null) => { setSelected(c); setName(c?.name || ""); setSend(c?.allow_send || false); setRemove(c?.allow_delete || false); setToken(""); setError(""); setDialog(kind); };
  return <section className="agent-connections" aria-labelledby="agent-connections-heading">
    <div className="workspace-section-heading"><div className="agent-heading"><Bot size={21} /><h2 ref={heading} tabIndex={-1} id="agent-connections-heading">Agent connections</h2></div><button onClick={() => open("create")}><Plus size={16} />Create connection</button></div>
    <p>Connect an assistant to all your email accounts and drafts. Choose separately whether it can send or delete mail.</p>
    {connections.isPending && <p role="status">Loading connections…</p>}
    {connections.isError && <p role="alert">Could not load connections. <button onClick={() => void connections.refetch()}>Reload connections</button></p>}
    {connections.data?.length === 0 && <p className="workspace-intro">No agent connections yet.</p>}
    <div className="agent-list">{connections.data?.map(c => <article key={c.id} aria-label={c.name} className="agent-card">
      <div><h3>{c.name}</h3><div className="agent-permissions"><span>Sending {c.allow_send ? "on" : "off"}</span><span>Deleting {c.allow_delete ? "on" : "off"}</span></div></div>
      <div className="agent-actions"><button aria-label={`Set up ${c.name}`} onClick={() => open("setup", c)}>Set up client</button><button aria-label={`Edit ${c.name}`} onClick={() => open("edit", c)}>Edit</button><button aria-label={`Revoke ${c.name}`} className="remove-account-link" onClick={() => open("revoke", c)}>Revoke</button></div>
    </article>)}</div>
    {(dialog === "create" || dialog === "edit") && <Modal title={dialog === "create" ? "Create agent connection" : "Edit agent connection"} onClose={close} initialFocus="#agent-name">
      <form className="setup agent-form" onSubmit={async event => {
        event.preventDefault(); if (busy) return; setBusy(true); setError("");
        try {
          if (dialog === "create") { const result = await agentConnectionsApi.create(name.trim()); update(result.connection); setSelected(result.connection); setToken(result.token); setDialog("setup"); }
          else if (selected) { update(await agentConnectionsApi.edit(selected.id, { name: name.trim(), allow_send: send, allow_delete: remove })); setDialog(null); }
        } catch { setError("Could not save the connection. Close this window and reload connections to check its current state."); }
        finally { setBusy(false); }
      }}>
        <label htmlFor="agent-name">Connection name<input id="agent-name" required maxLength={100} value={name} onChange={e => setName(e.target.value)} disabled={busy} autoComplete="off" placeholder="For example, Research assistant" /></label>
        <p>Access includes every account you connect later and all your drafts.</p>
        {dialog === "create" ? <p>Sending and deleting start off. You can enable either after creating the connection.</p> : <div className="agent-controls">
          <label><input type="checkbox" aria-label="Allow sending" aria-describedby="agent-send-note" checked={send} onChange={e => setSend(e.target.checked)} disabled={busy} /><span>Allow sending<small id="agent-send-note">New mail, replies, forwards and drafts.</small></span></label>
          <label><input type="checkbox" aria-label="Allow deleting" aria-describedby="agent-delete-note" checked={remove} onChange={e => setRemove(e.target.checked)} disabled={busy} /><span>Allow deleting<small id="agent-delete-note">Trash, permanent deletion, empty Trash, folders and drafts.</small></span></label>
        </div>}
        {error && <p role="alert">{error}</p>}
        <footer><button type="button" disabled={busy} onClick={close}>Cancel</button><button className="primary" disabled={busy || !name.trim()}>{busy ? "Saving…" : dialog === "create" ? "Create" : "Save changes"}</button></footer>
      </form>
    </Modal>}
    {dialog === "setup" && <Modal key="setup" title={token ? "Connection created" : `Set up ${selected?.name || "client"}`} onClose={close} initialFocus="[data-agent-setup]">
      <div className="agent-setup"><ClientSetup token={token} /><footer><button className="primary" onClick={close}>Done</button></footer></div>
    </Modal>}
    {dialog === "revoke" && selected && <Modal title={`Revoke ${selected.name}?`} onClose={close}>
      <div className="agent-setup"><p>This disconnects clients using this connection, including browser-authorized clients. Your other connections stay active.</p>{error && <p role="alert">{error}</p>}<footer><button disabled={busy} onClick={close}>Keep connection</button><button className="destructive" disabled={busy} onClick={async () => {
        setBusy(true); setError(""); try { await agentConnectionsApi.revoke(selected.id); client.setQueryData<AgentConnection[]>(key, old => old?.filter(c => c.id !== selected.id)); setDialog(null); requestAnimationFrame(() => heading.current?.focus()); } catch { setError("Could not confirm revocation. Reload connections to check its current state."); } finally { setBusy(false); }
      }}>{busy ? "Revoking…" : "Revoke connection"}</button></footer></div>
    </Modal>}
  </section>;
}
function ClientSetup({ token }: { token: string }) {
  const [client, setClient] = useState("claude");
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const endpoint = `${location.origin}/mcp`;
  return <>
    {token && <div className="agent-token"><h3>Save your token</h3><p>This is the only time Dakia shows it. Keep it private.</p><code>{token}</code><button onClick={async () => { try { await navigator.clipboard.writeText(token); setCopied(true); setCopyError(false); } catch { setCopyError(true); } }}>{copied ? "Token copied" : "Copy token"}</button>{copyError && <p role="alert">Select the token above and copy it manually.</p>}</div>}
    <label htmlFor="agent-client">Client</label><select data-agent-setup id="agent-client" value={client} onChange={e => setClient(e.target.value)}><option value="claude">Claude Code</option><option value="chatgpt">ChatGPT</option><option value="generic">Other MCP clients</option></select>
    <h3>Server URL</h3><code className="agent-endpoint">{endpoint}</code>
    {client === "claude" && <><p>Add the HTTP server, then use <code>/mcp</code> in Claude Code to authorize it in your browser. Choose this connection in Dakia.</p><pre>{`claude mcp add --transport http dakia ${endpoint}`}</pre><p>For token-based setup, use your saved connection token:</p><pre>{`claude mcp add --transport http dakia ${endpoint} \\\n  --header "Authorization: Bearer YOUR_TOKEN"`}</pre></>}
    {client === "chatgpt" && <ol><li>Open ChatGPT on the web. Go to Plugins, choose Add custom MCP server, and enter the server URL above.</li><li>Choose OAuth and dynamic client registration. Your Dakia URL must be reachable by ChatGPT over HTTPS.</li><li>Continue to Dakia, sign in and choose this agent connection. Select Connect.</li><li>Create and install the plugin, then select it with @ in a conversation.</li></ol>}
    {client === "generic" && <><p>Use Streamable HTTP with browser OAuth, or add this header using your saved token:</p><pre>Authorization: Bearer YOUR_TOKEN</pre><p>OAuth metadata is discovered from the server’s authentication challenge. The flow uses S256 PKCE, dynamic public-client registration and the server URL as the resource.</p></>}
  </>;
}
