import { useQuery } from "@tanstack/react-query";
import type { Account, Draft, Submission } from "@dakia/api-types";
import { extraApi } from "./extra-api";
import { Empty, ErrorNotice } from "./ui";
import { scheduledTime, submissionLabel } from "./scheduled";
export function DraftList({
  accounts,
  onOpen,
}: {
  accounts: Account[];
  onOpen: (draft: Draft) => void;
}) {
  const query = useQuery({
    queryKey: ["drafts", ...accounts.map((a) => a.id)],
    queryFn: async () =>
      Promise.all(
        accounts.map(async (account) => ({
          account,
          rows: await extraApi
            .drafts(account.id)
            .catch((error) => error as Error),
        })),
      ),
  });
  return (
    <div className="record-list">
      <ErrorNotice error={query.error} />
      {query.isPending ? (
        <Empty title="Loading drafts…" />
      ) : (
        query.data?.map(({ account, rows }) => (
          <section key={account.id}>
            <h2>{account.display_name}</h2>
            {rows instanceof Error ? (
              <ErrorNotice error={rows} />
            ) : rows.length ? (
              rows.map((d) => (
                <button title={`Open draft: ${d.subject || "(No subject)"}`}
                  className="draft-row"
                  key={d.id}
                  onClick={() => onOpen(d)}
                >
                  <b>{d.subject || "(No subject)"}</b>
                  <span>
                    {d.to.map((a) => a.email).join(", ") || "No recipients"}
                  </span>
                  <small>{new Date(d.saved_at).toLocaleString()}</small>
                </button>
              ))
            ) : (
              <p>No drafts</p>
            )}
          </section>
        ))
      )}
    </div>
  );
}
export function ScheduledList({
  accounts,
  onSelect,
}: {
  accounts: Account[];
  onSelect: (s: Submission) => void;
}) {
  const query = useQuery({
    queryKey: ["submissions", ...accounts.map((a) => a.id)],
    refetchInterval: 15000,
    queryFn: async () =>
      Promise.all(
        accounts.map(async (account) => ({
          account,
          rows: await extraApi
            .scheduled(account.id)
            .catch((error) => error as Error),
        })),
      ),
  });
  return (
    <div className="record-list scheduled-list">
      <ErrorNotice error={query.error} />
      {query.isPending ? <Empty title="Loading scheduled mail…" /> : <>
        {query.data?.every(({ rows }) => !(rows instanceof Error) && !rows.length) && <Empty title="No scheduled messages"><p>Messages you send later will appear here.</p></Empty>}
        {query.data?.map(({ account, rows }) => rows instanceof Error ? <section key={account.id}><h2>{account.display_name}</h2><ErrorNotice error={rows} /></section> : rows.length > 0 && <section key={account.id}>
          <h2>{account.display_name}</h2>
          {rows.map(({ submission: s, subject, to }) => <article className="scheduled-card" key={s.id}>
            <h3>{subject || "(No subject)"}</h3>
            <p className="scheduled-recipient">To {to.map((a) => a.name || a.email).join(", ") || "No recipients"}</p>
            <p className="scheduled-time"><time dateTime={s.due_at}>{scheduledTime(s)}</time><span>{s.timezone.replaceAll("_", " ")}</span></p>
            <p className="scheduled-state">{submissionLabel(s)}</p>
            <button title={["scheduled", "claimed"].includes(s.state) ? "Manage schedule" : "Review delivery"} onClick={() => onSelect(s)}>{["scheduled", "claimed"].includes(s.state) ? "Manage schedule" : "Review delivery"}</button>
          </article>)}
        </section>)}
      </>}
    </div>
  );
}
