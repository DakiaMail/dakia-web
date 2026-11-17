-- A local accelerator can discard acknowledged changes only after its atomic
-- snapshot is durable. Other consumers behind this floor rebuild from chunks.
CREATE TABLE vector_replay (id INTEGER PRIMARY KEY CHECK(id=1),floor INTEGER NOT NULL DEFAULT 0);
INSERT INTO vector_replay VALUES(1,0);
