-- Rank/filter compact identities before fetching JSON or body overflow pages.
-- SQLite permits long text keys; PostgreSQL deliberately uses its native
-- indexes and heap/TOAST separation instead of this unbounded-text B-tree.
CREATE INDEX documents_rank_filter ON documents(
 account_id,message_id,received_at,source,state,sender,recipients,subject,
 participants,content_version,received_date,unread,starred,category,snoozed_until
);
