SELECT rel.relname::text, con.conname::text, fact.attribute, fact.value
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class rel ON rel.oid = con.conrelid
JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
LEFT JOIN pg_catalog.pg_class target ON target.oid = con.confrelid
LEFT JOIN pg_catalog.pg_namespace target_ns ON target_ns.oid = target.relnamespace
CROSS JOIN LATERAL (VALUES
    ('kind', con.contype::text),
    ('columns', ARRAY(SELECT col.attname FROM unnest(con.conkey) WITH ORDINALITY key(num, pos)
        JOIN pg_catalog.pg_attribute col ON col.attrelid = rel.oid AND col.attnum = key.num ORDER BY key.pos)::text),
    ('target', CASE WHEN target.oid IS NOT NULL THEN
        format('%I.%I', CASE WHEN target_ns.nspname = $1 THEN '' ELSE target_ns.nspname END, target.relname) END),
    ('target_columns', ARRAY(SELECT col.attname FROM unnest(con.confkey) WITH ORDINALITY key(num, pos)
        JOIN pg_catalog.pg_attribute col ON col.attrelid = target.oid AND col.attnum = key.num ORDER BY key.pos)::text),
    ('delete_action', con.confdeltype::text),
    ('update_action', con.confupdtype::text),
    ('match', con.confmatchtype::text),
    ('deferrable', con.condeferrable::text),
    ('deferred', con.condeferred::text),
    ('valid', con.convalidated::text),
    ('expression', pg_catalog.pg_get_expr(con.conbin, con.conrelid))
) fact(attribute, value)
WHERE ns.nspname = $1
