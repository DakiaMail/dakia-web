import { CopyButton } from "@dakia/ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "./api";

export function Invitations({ userId }: { userId: string }) {
  const client = useQueryClient();
  const access = useQuery({ queryKey: ["local-access", userId], queryFn: api.localAccess, retry: false });
  const invites = useQuery({ queryKey: ["invitations", userId], queryFn: api.invitations, enabled: access.data?.can_invite === true, retry: false });
  const [link, setLink] = useState<{ id: string; url: string } | null>(null);
  const create = useMutation({ mutationFn: api.createInvitation, onSuccess: value => { setLink(value); void client.invalidateQueries({ queryKey: ["invitations", userId] }); } });
  const revoke = useMutation({ mutationFn: api.revokeInvitation, onSuccess: (_, id) => { if (link?.id === id) setLink(null); void client.invalidateQueries({ queryKey: ["invitations", userId] }); } });
  if (!access.data?.can_invite) return null;
  return <section aria-labelledby="workspace-invitations" className="workspace-invitations">
    <div className="workspace-section-heading"><h2 id="workspace-invitations">Invite someone</h2><button className="primary" disabled={create.isPending} onClick={() => create.mutate()}>{create.isPending ? "Creating…" : "Create invitation"}</button></div>
    <p>They’ll get their own login and separate email accounts. Each invitation works once and expires after 24 hours.</p>
    {link && <div className="invitation-link"><label>Invitation link<input aria-label="Invitation link" readOnly value={link.url} onFocus={event => event.target.select()} /></label><CopyButton value={link.url}>Copy link</CopyButton></div>}
    {(create.isError || revoke.isError || invites.isError) && <p role="alert">Couldn’t update invitations. Try again. {invites.isError && <button onClick={() => void invites.refetch()}>Retry</button>}</p>}
    {!!invites.data?.length && <ul className="invitation-list">{invites.data.map(invite => <li key={invite.id}><span>Expires {new Date(invite.expires_at * 1000).toLocaleString()}</span><button disabled={revoke.isPending} onClick={() => revoke.mutate(invite.id)}>Revoke invitation</button></li>)}</ul>}
  </section>;
}
