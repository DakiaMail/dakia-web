UPDATE search_documents SET has_attachment=CASE
 WHEN json_array_length(attachment_details)>0 THEN 1
 WHEN body_state='ready' THEN 0 ELSE NULL END WHERE has_attachment IS NULL;
CREATE INDEX search_attachment_pending ON search_documents(message_id) WHERE has_attachment IS NULL;
