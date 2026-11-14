-- Metadata-only pages retain date order and read full documents only for
-- admitted rows. Dense participant lists do not require a global posting scan.
CREATE INDEX documents_filter_order ON documents(
 account_id,received_at DESC,message_id DESC,sender,recipients,subject,source,
 content_version,state,received_date,unread,starred,category,snoozed_until
) WHERE state<>'deleted';
