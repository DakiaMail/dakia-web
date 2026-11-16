ALTER TABLE search_documents ADD COLUMN has_attachment INTEGER;
UPDATE search_documents SET has_attachment=CASE
 WHEN json_array_length(attachment_details::json)>0 THEN 1
 WHEN body_state='ready' THEN 0 ELSE NULL END;
CREATE INDEX search_attachment_pending ON search_documents(account_id,message_id) WHERE has_attachment IS NULL;
