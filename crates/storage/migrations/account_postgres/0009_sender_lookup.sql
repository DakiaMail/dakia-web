ALTER TABLE messages ADD COLUMN sender_address TEXT GENERATED ALWAYS AS (document::jsonb->'sender'->>'email') STORED;
CREATE INDEX messages_sender_id ON messages(account_id,sender_address,id COLLATE "C");
CREATE INDEX operations_prefix ON operations(account_id,id COLLATE "C");
-- Match the ISO date forms used by mailbox metadata and frozen cleanup cutoffs.
-- Opaque/invalid legacy dates are absent, not SQL cast errors. Fix UTC and date
-- style explicitly instead of inheriting a connection's locale or timezone.
CREATE FUNCTION mail_julianday(value TEXT) RETURNS DOUBLE PRECISION LANGUAGE plpgsql STABLE STRICT SET timezone='UTC' SET datestyle='ISO,YMD' AS $$
BEGIN
 IF value !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}([Tt ][0-9]{2}:[0-9]{2}(:[0-9]{2}([.][0-9]+)?)?([Zz]|[+-][0-9]{2}:[0-9]{2})?)?$' THEN RETURN NULL; END IF;
 RETURN (round(extract(epoch FROM value::TIMESTAMPTZ)*1000)/1000)::DOUBLE PRECISION/86400+2440587.5;
EXCEPTION WHEN invalid_datetime_format OR datetime_field_overflow THEN RETURN NULL;
END $$;
