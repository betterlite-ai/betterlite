SELECT typ.typname::text, ''::text, 'labels'::text, array_agg(label.enumlabel ORDER BY label.enumsortorder)::text
FROM pg_catalog.pg_type typ
JOIN pg_catalog.pg_namespace ns ON ns.oid = typ.typnamespace
JOIN pg_catalog.pg_enum label ON label.enumtypid = typ.oid
WHERE ns.nspname = $1
GROUP BY typ.typname
