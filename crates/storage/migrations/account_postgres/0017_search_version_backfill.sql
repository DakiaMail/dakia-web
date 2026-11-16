DROP INDEX search_delivery_upgrade;
CREATE INDEX search_delivery_upgrade ON search_documents(account_id,message_id COLLATE "C") WHERE delivered_version=0 AND delivered=1;
