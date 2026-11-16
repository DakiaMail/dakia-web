-- Match the existing SQLite metadata preview's ASCII case folding. Full indexed
-- content uses shared Unicode normalization and database-native lexical search.
CREATE FUNCTION mail_search_contains(value TEXT,needle TEXT) RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE SET search_path=dakia_mail,pg_catalog AS $$
 SELECT strpos(translate(value,'ABCDEFGHIJKLMNOPQRSTUVWXYZ','abcdefghijklmnopqrstuvwxyz'),translate(needle,'ABCDEFGHIJKLMNOPQRSTUVWXYZ','abcdefghijklmnopqrstuvwxyz'))>0
$$;
