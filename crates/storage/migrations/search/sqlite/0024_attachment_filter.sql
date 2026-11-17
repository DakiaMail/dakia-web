-- Attachment presence also participates in the in-memory candidate filter.
CREATE TRIGGER vector_attachment_changed AFTER UPDATE OF attachment_details ON documents
WHEN (json_array_length(OLD.attachment_details)>0)<>(json_array_length(NEW.attachment_details)>0) BEGIN
 UPDATE documents SET filter_revision=filter_revision+1 WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_changes(chunk_id,account_id,model_revision,metadata_only)
 SELECT id,account_id,model_revision,1 FROM chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
END;
