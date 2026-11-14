CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE TABLE documents (
 account_id TEXT NOT NULL, message_id TEXT NOT NULL, version BIGINT NOT NULL,
 source TEXT NOT NULL, metadata TEXT NOT NULL,
 sender TEXT NOT NULL, recipients TEXT NOT NULL, subject TEXT NOT NULL,
 body TEXT NOT NULL, participants TEXT NOT NULL, attachment_text TEXT NOT NULL,
 received_at TEXT NOT NULL,state TEXT NOT NULL,
 lexemes tsvector GENERATED ALWAYS AS (
  setweight(to_tsvector('simple'::regconfig,sender || ' ' || recipients || ' ' || participants),'A') ||
  setweight(to_tsvector('simple'::regconfig,subject),'A') ||
  setweight(to_tsvector('simple'::regconfig,body || ' ' || attachment_text),'B')
 ) STORED,
 PRIMARY KEY(account_id,message_id)
);
CREATE INDEX documents_account_order ON documents(account_id,received_at DESC,message_id DESC);
CREATE INDEX documents_terms ON documents USING gin(lexemes);
CREATE INDEX documents_subject_typos ON documents USING gin(subject gin_trgm_ops);
CREATE TABLE chunks (
 id BIGSERIAL PRIMARY KEY, account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 source TEXT NOT NULL,model_revision TEXT NOT NULL,ordinal BIGINT NOT NULL,text TEXT NOT NULL,
 embedding vector(384) NOT NULL,UNIQUE(account_id,message_id,model_revision,ordinal)
);
CREATE INDEX chunks_message ON chunks(account_id,message_id);
CREATE INDEX chunks_ann ON chunks USING hnsw(embedding vector_cosine_ops);
CREATE TABLE index_identity (id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);
INSERT INTO index_identity VALUES(1,gen_random_uuid()::text);
