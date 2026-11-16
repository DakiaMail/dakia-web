-- Keep small subject keys together. A mixed-field covering index would put
-- body overflow pages on the lookup path for every subject candidate.
-- The retained canonical text proves the same complete ranking head without
-- fetching wide lexical rows. Other fields keep their existing access paths.
CREATE INDEX lexical_subject_head ON lexical_chunks(
 account_id,message_id,field,content_version,revision,text
) WHERE field IN (2) AND native_text<>'';
