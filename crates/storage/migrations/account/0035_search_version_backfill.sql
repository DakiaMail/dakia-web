DROP INDEX search_delivery_upgrade;
CREATE INDEX search_delivery_upgrade ON search_documents(message_id) WHERE delivered_version=0 AND delivered=1;
