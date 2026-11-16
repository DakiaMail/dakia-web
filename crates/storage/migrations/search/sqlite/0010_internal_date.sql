ALTER TABLE documents ADD COLUMN received_date TEXT;
UPDATE documents SET received_date=json_extract(metadata,'$.received_date');
CREATE INDEX search_received_day ON documents(account_id,received_date,message_id) WHERE state<>'deleted';
CREATE INDEX search_missing_received_day ON documents(account_id) WHERE state<>'deleted' AND received_date IS NULL;
