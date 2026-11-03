SELECT rel.relname::text, ''::text, fact.attribute, fact.value
FROM pg_catalog.pg_sequence seq
JOIN pg_catalog.pg_class rel ON rel.oid = seq.seqrelid
JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
JOIN pg_catalog.pg_type typ ON typ.oid = seq.seqtypid
LEFT JOIN pg_catalog.pg_depend dep ON dep.classid = 'pg_catalog.pg_class'::regclass
    AND dep.objid = rel.oid AND dep.refclassid = 'pg_catalog.pg_class'::regclass
    AND dep.deptype IN ('a', 'i')
LEFT JOIN pg_catalog.pg_class owner ON owner.oid = dep.refobjid
LEFT JOIN pg_catalog.pg_namespace owner_ns ON owner_ns.oid = owner.relnamespace
LEFT JOIN pg_catalog.pg_attribute col ON col.attrelid = owner.oid AND col.attnum = dep.refobjsubid
CROSS JOIN LATERAL (VALUES
    ('type', typ.typname::text),
    ('start', seq.seqstart::text),
    ('increment', seq.seqincrement::text),
    ('min', seq.seqmin::text),
    ('max', seq.seqmax::text),
    ('cache', seq.seqcache::text),
    ('cycle', seq.seqcycle::text),
    ('owned_relation', CASE WHEN owner.oid IS NOT NULL THEN
        format('%I.%I', CASE WHEN owner_ns.nspname = $1 THEN '' ELSE owner_ns.nspname END, owner.relname) END),
    ('owned_column', col.attname::text)
) fact(attribute, value)
WHERE ns.nspname = $1
