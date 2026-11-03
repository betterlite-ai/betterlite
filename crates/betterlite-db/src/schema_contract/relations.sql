SELECT rel.relname::text, ''::text, 'kind'::text, rel.relkind::text
FROM pg_catalog.pg_class rel
JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
WHERE ns.nspname = $1 AND rel.relkind IN ('r', 'p', 'v', 'm', 'f')
