-- Add a matching index without removing the previous index during rollout.
-- Match Rust identity ties and SQLite BINARY order regardless of database locale.
CREATE INDEX documents_account_order_binary ON documents(account_id,received_at DESC,message_id COLLATE "C" DESC);
