SELECT rel.relname::text, idx.relname::text, fact.attribute, fact.value
FROM pg_catalog.pg_index ind
JOIN pg_catalog.pg_class rel ON rel.oid = ind.indrelid
JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
JOIN pg_catalog.pg_class idx ON idx.oid = ind.indexrelid
JOIN pg_catalog.pg_am method ON method.oid = idx.relam
CROSS JOIN LATERAL (VALUES
    ('method', method.amname::text),
    ('keys', ARRAY(SELECT pg_catalog.pg_get_indexdef(idx.oid, pos, false)
        FROM generate_series(1, ind.indnatts) pos ORDER BY pos)::text),
    ('key_count', ind.indnkeyatts::text),
    ('direction', ind.indoption::text),
    ('unique', ind.indisunique::text),
    ('nulls_not_distinct', ind.indnullsnotdistinct::text),
    ('valid', ind.indisvalid::text),
    ('predicate', pg_catalog.pg_get_expr(ind.indpred, ind.indrelid))
) fact(attribute, value)
WHERE ns.nspname = $1
