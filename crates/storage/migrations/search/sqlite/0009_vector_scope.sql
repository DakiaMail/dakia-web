-- Healthy accounts must not wait for another account/model's replay backlog.
CREATE INDEX embedding_changes_scope ON embedding_changes(account_id,model_revision,seq);
