use deadpool_postgres::GenericClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub name: String,
    pub schema: String,
    pub partitioned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexState {
    pub valid: bool,
    pub table: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivalentIndex {
    pub name: String,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableKind {
    pub partitioned: bool,
}

pub fn regclass_name(schema: &str, name: &str) -> String {
    let quoted_schema = schema.replace('"', "\"\"");
    let quoted_name = name.replace('"', "\"\"");
    format!("\"{quoted_schema}\".\"{quoted_name}\"")
}

pub async fn table_kind<C: GenericClient + Sync>(
    client: &C,
    schema: &str,
    table: &str,
) -> Result<Option<TableKind>, tokio_postgres::Error> {
    let row = client
        .query_opt(
            "SELECT c.relkind = 'p' AS partitioned FROM pg_class c WHERE c.oid = to_regclass($1)",
            &[&regclass_name(schema, table)],
        )
        .await?;
    Ok(row.map(|row| TableKind {
        partitioned: row.get("partitioned"),
    }))
}

pub async fn index_state<C: GenericClient + Sync>(
    client: &C,
    schema: &str,
    index: &str,
) -> Result<Option<IndexState>, tokio_postgres::Error> {
    let row = client
        .query_opt(
            "SELECT x.indisvalid AS valid, t.relname AS \"table\" FROM pg_index x JOIN pg_class t ON t.oid = x.indrelid WHERE x.indexrelid = to_regclass($1)",
            &[&regclass_name(schema, index)],
        )
        .await?;
    Ok(row.map(|row| IndexState {
        valid: row.get("valid"),
        table: row.get("table"),
    }))
}

pub async fn equivalent_indexes<C: GenericClient + Sync>(
    client: &C,
    schema: &str,
    table: &str,
    expected_name: &str,
    index: &crate::request_log_index_catalog::RequestLogIndex<'_>,
) -> Result<Vec<EquivalentIndex>, tokio_postgres::Error> {
    let columns = index.columns();
    let rowset = client
        .query(
            "SELECT i.relname AS name, x.indisvalid AS valid FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid JOIN pg_am am ON am.oid = i.relam WHERE x.indrelid = to_regclass($1) AND i.relname <> $2 AND am.amname = 'btree' AND NOT x.indisunique AND x.indexprs IS NULL AND x.indpred IS NULL AND x.indnkeyatts = x.indnatts AND NOT EXISTS (SELECT 1 FROM unnest(x.indoption::int2[]) o WHERE o <> 0) AND NOT EXISTS (SELECT 1 FROM unnest(x.indclass::oid[]) c JOIN pg_opclass oc ON oc.oid = c WHERE NOT oc.opcdefault) AND NOT EXISTS (SELECT 1 FROM unnest(x.indcollation::oid[]) WITH ORDINALITY c(coll, ord) JOIN unnest(x.indkey::int2[]) WITH ORDINALITY k(attnum, ord) ON k.ord = c.ord JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.attnum WHERE c.coll <> 0 AND c.coll <> a.attcollation) AND (SELECT array_agg(a.attname::text ORDER BY k.ord) FROM unnest(x.indkey::int2[]) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.attnum) = $3::text[] AND NOT EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = x.indexrelid) ORDER BY x.indisvalid DESC, i.relname",
            &[&regclass_name(schema, table), &expected_name, &columns],
        )
        .await?;
    Ok(rowset
        .into_iter()
        .map(|row| EquivalentIndex {
            name: row.get("name"),
            valid: row.get("valid"),
        })
        .collect())
}

pub async fn children_without_the_index<C: GenericClient + Sync>(
    client: &C,
    schema: &str,
    table: &str,
    index: &str,
) -> Result<Vec<Relation>, tokio_postgres::Error> {
    let rowset = client
        .query(
            "SELECT child.relname AS name, n.nspname AS schema, child.relkind = 'p' AS partitioned FROM pg_inherits i JOIN pg_class child ON child.oid = i.inhrelid JOIN pg_namespace n ON n.oid = child.relnamespace WHERE i.inhparent = to_regclass($1) AND NOT EXISTS (SELECT 1 FROM pg_inherits attached JOIN pg_index x ON x.indexrelid = attached.inhrelid WHERE attached.inhparent = to_regclass($2) AND x.indrelid = child.oid) ORDER BY child.relname",
            &[&regclass_name(schema, table), &regclass_name(schema, index)],
        )
        .await?;
    Ok(rowset
        .into_iter()
        .map(|row| Relation {
            name: row.get("name"),
            schema: row.get("schema"),
            partitioned: row.get("partitioned"),
        })
        .collect())
}
