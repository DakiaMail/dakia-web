-- Display updates and topology check timestamps do not change membership.
DROP TRIGGER thread_message_update;
CREATE TRIGGER thread_message_update AFTER UPDATE OF document ON messages
WHEN json_extract(OLD.document,'$.provider_id') IS NOT json_extract(NEW.document,'$.provider_id')
  OR json_extract(OLD.document,'$.folder_ids') IS NOT json_extract(NEW.document,'$.folder_ids')
  OR json_extract(OLD.document,'$.thread_id') IS NOT json_extract(NEW.document,'$.thread_id')
BEGIN UPDATE thread_generation SET revision=revision+1; END;
DROP TRIGGER thread_topology_update;
CREATE TRIGGER thread_topology_update AFTER UPDATE OF document ON provider_documents
WHEN (NEW.key='thread:folders' AND json_extract(OLD.document,'$.names') IS NOT json_extract(NEW.document,'$.names'))
  OR (substr(NEW.key,1,13)='thread:epoch:' AND OLD.document<>NEW.document)
  OR (NEW.key='probe' AND json_extract(OLD.document,'$.full_search_coverage') IS NOT json_extract(NEW.document,'$.full_search_coverage'))
BEGIN UPDATE thread_generation SET revision=revision+1; END;
-- Older proofs used a different revision meaning.
DELETE FROM thread_verifications;
