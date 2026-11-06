-- Bound local inbox ordering and pending-intent lookups without copying headers.
CREATE INDEX messages_inbox_received_order ON messages(json_extract(document,'$.received_at') DESC, id DESC)
WHERE instr(upper(json_extract(document,'$.folder_ids')),'"INBOX"')>0;
CREATE INDEX operations_pending_target_kind ON operations(json_extract(document,'$.target_id'), json_extract(document,'$.status'), json_extract(document,'$.change.kind'));
