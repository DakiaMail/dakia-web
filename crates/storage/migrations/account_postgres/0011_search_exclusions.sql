CREATE TABLE search_excluded_folders (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 name TEXT NOT NULL,PRIMARY KEY(account_id,name)
);
CREATE FUNCTION search_default_exclusions() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 INSERT INTO search_excluded_folders(account_id,name) SELECT NEW.account_id,unnest(ARRAY['Trash','Spam','Junk','[Gmail]/Trash','[Gmail]/Spam']);
 RETURN NEW;
END $$;
CREATE TRIGGER search_default_exclusions AFTER INSERT ON account_locks FOR EACH ROW EXECUTE FUNCTION search_default_exclusions();
INSERT INTO search_excluded_folders(account_id,name) SELECT account_id,unnest(ARRAY['Trash','Spam','Junk','[Gmail]/Trash','[Gmail]/Spam']) FROM account_locks;
CREATE VIEW search_eligible_messages AS SELECT m.account_id,m.id FROM messages m WHERE cardinality(m.folder_ids)>0 AND NOT EXISTS(SELECT 1 FROM unnest(m.folder_ids) f JOIN search_excluded_folders e ON e.account_id=m.account_id AND lower(f)=lower(e.name)) AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch);
