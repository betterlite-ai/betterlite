SELECT rel.relname::text, col.attname::text, fact.attribute, fact.value
FROM pg_catalog.pg_class rel
JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
JOIN pg_catalog.pg_attribute col ON col.attrelid = rel.oid
JOIN pg_catalog.pg_type typ ON typ.oid = col.atttypid
JOIN pg_catalog.pg_namespace type_ns ON type_ns.oid = typ.typnamespace
LEFT JOIN pg_catalog.pg_attrdef def ON def.adrelid = rel.oid AND def.adnum = col.attnum
LEFT JOIN LATERAL (
    SELECT seq.relname, seq.oid, seq_ns.nspname
    FROM pg_catalog.pg_depend dep
    JOIN pg_catalog.pg_class seq ON seq.oid = dep.refobjid AND seq.relkind = 'S'
    JOIN pg_catalog.pg_namespace seq_ns ON seq_ns.oid = seq.relnamespace
    WHERE dep.classid = 'pg_catalog.pg_attrdef'::regclass AND dep.objid = def.oid
      AND dep.refclassid = 'pg_catalog.pg_class'::regclass
      AND pg_catalog.pg_get_expr(def.adbin, def.adrelid) =
          format('nextval(%L::regclass)', seq.oid::regclass::text)
) serial ON true
CROSS JOIN LATERAL (SELECT pg_catalog.pg_get_expr(def.adbin, def.adrelid) AS expression) expr
CROSS JOIN LATERAL (VALUES
    ('position', col.attnum::text),
    ('type', format('%I.%I', CASE WHEN type_ns.nspname = $1 THEN '' ELSE type_ns.nspname END, typ.typname)),
    ('typmod', col.atttypmod::text),
    ('not_null', col.attnotnull::text),
    ('identity', col.attidentity::text),
    ('generated', col.attgenerated::text),
    ('default', CASE WHEN serial.oid IS NOT NULL THEN
        format('nextval(%I.%I)', CASE WHEN serial.nspname = $1 THEN '' ELSE serial.nspname END, serial.relname)
      WHEN typ.typtype = 'e' AND right(expr.expression, length(typ.oid::regtype::text) + 2) = '::' || typ.oid::regtype::text THEN
        left(expr.expression, length(expr.expression) - length(typ.oid::regtype::text) - 2)
          || format('::%I.%I', CASE WHEN type_ns.nspname = $1 THEN '' ELSE type_ns.nspname END, typ.typname)
      ELSE expr.expression END)
) fact(attribute, value)
WHERE ns.nspname = $1 AND rel.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND col.attnum > 0 AND NOT col.attisdropped
