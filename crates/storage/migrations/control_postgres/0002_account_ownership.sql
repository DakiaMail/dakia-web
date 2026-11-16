-- Composite identity is used by account-scoped application tables.
ALTER TABLE accounts ADD CONSTRAINT accounts_owner_identity UNIQUE(id,user_id);
