use crate::request_log_index_catalog::definitions::RequestLogIndex;
use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

const TABLE_KIND_QUERY: &str =
    "SELECT c.relkind = 'p' AS partitioned FROM pg_class c WHERE c.oid = to_regclass($1)";
const INDEX_STATE_QUERY: &str = "SELECT x.indisvalid AS valid, t.relname AS \"table\" FROM pg_index x JOIN pg_class t ON t.oid = x.indrelid WHERE x.indexrelid = to_regclass($1)";
const EQUIVALENT_INDEXES_QUERY: &str = "SELECT i.relname AS name, x.indisvalid AS valid FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid JOIN pg_am am ON am.oid = i.relam WHERE x.indrelid = to_regclass($1) AND i.relname <> $2 AND am.amname = 'btree' AND NOT x.indisunique AND x.indexprs IS NULL AND x.indpred IS NULL AND x.indnkeyatts = x.indnatts AND NOT EXISTS (SELECT 1 FROM unnest(x.indoption::int2[]) o WHERE o <> 0) AND NOT EXISTS (SELECT 1 FROM unnest(x.indclass::oid[]) c JOIN pg_opclass oc ON oc.oid = c WHERE NOT oc.opcdefault) AND NOT EXISTS (SELECT 1 FROM unnest(x.indcollation::oid[]) WITH ORDINALITY c(coll, ord) JOIN unnest(x.indkey::int2[]) WITH ORDINALITY k(attnum, ord) ON k.ord = c.ord JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.attnum WHERE c.coll <> 0 AND c.coll <> a.attcollation) AND (SELECT array_agg(a.attname::text ORDER BY k.ord) FROM unnest(x.indkey::int2[]) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.attnum) = $3::text[] AND NOT EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = x.indexrelid) ORDER BY x.indisvalid DESC, i.relname";
const CHILDREN_QUERY: &str = "SELECT child.relname AS name, n.nspname AS schema, child.relkind = 'p' AS partitioned FROM pg_inherits i JOIN pg_class child ON child.oid = i.inhrelid JOIN pg_namespace n ON n.oid = child.relnamespace WHERE i.inhparent = to_regclass($1) AND NOT EXISTS (SELECT 1 FROM pg_inherits attached JOIN pg_index x ON x.indexrelid = attached.inhrelid WHERE attached.inhparent = to_regclass($2) AND x.indrelid = child.oid) ORDER BY child.relname";

#[derive(Debug, thiserror::Error)]
pub enum RequestLogIndexCatalogError {
    #[error("request-log index catalog query failed")]
    Query(
        #[source]
        #[from]
        tokio_postgres::Error,
    ),
}

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

#[must_use]
pub fn regclass_name(schema: &str, name: &str) -> String {
    let quoted_schema = schema.replace('"', "\"\"");
    let quoted_name = name.replace('"', "\"\"");
    format!("\"{quoted_schema}\".\"{quoted_name}\"")
}

pub async fn table_kind<C>(
    client: &C,
    schema: &str,
    table: &str,
) -> Result<Option<TableKind>, RequestLogIndexCatalogError>
where
    C: GenericClient + Sync,
{
    let identity = regclass_name(schema, table);
    let params: [&(dyn ToSql + Sync); 1] = [&identity];
    let row = client.query_opt(TABLE_KIND_QUERY, &params).await?;
    row.map(|table_row| {
        let partitioned = table_row.get("partitioned");
        Ok(TableKind { partitioned })
    })
    .transpose()
}

pub async fn index_state<C>(
    client: &C,
    schema: &str,
    index: &str,
) -> Result<Option<IndexState>, RequestLogIndexCatalogError>
where
    C: GenericClient + Sync,
{
    let identity = regclass_name(schema, index);
    let params: [&(dyn ToSql + Sync); 1] = [&identity];
    let row = client.query_opt(INDEX_STATE_QUERY, &params).await?;
    let Some(index_row) = row else {
        return Ok(None);
    };
    let valid = index_row.get("valid");
    let table = index_row.get("table");
    Ok(Some(IndexState { valid, table }))
}

pub async fn equivalent_indexes<C>(
    client: &C,
    schema: &str,
    table: &str,
    expected_name: &str,
    index: &RequestLogIndex<'_>,
) -> Result<Vec<EquivalentIndex>, RequestLogIndexCatalogError>
where
    C: GenericClient + Sync,
{
    let columns = index.columns();
    let identity = regclass_name(schema, table);
    let params: [&(dyn ToSql + Sync); 3] = [&identity, &expected_name, &columns];
    let rowset = client.query(EQUIVALENT_INDEXES_QUERY, &params).await?;
    if rowset.is_empty() {
        return Ok(Vec::new());
    }
    let equivalent = rowset.iter().fold(Vec::new(), |mut indexes, row| {
        indexes.push(EquivalentIndex {
            name: row.get("name"),
            valid: row.get("valid"),
        });
        indexes
    });
    Ok(equivalent)
}

pub async fn children_without_the_index<C>(
    client: &C,
    schema: &str,
    table: &str,
    index: &str,
) -> Result<Vec<Relation>, RequestLogIndexCatalogError>
where
    C: GenericClient + Sync,
{
    let table_identity = regclass_name(schema, table);
    let index_identity = regclass_name(schema, index);
    let params: [&(dyn ToSql + Sync); 2] = [&table_identity, &index_identity];
    let rowset = client.query(CHILDREN_QUERY, &params).await?;
    let mut children = Vec::with_capacity(rowset.len());
    for row in &rowset {
        children.push(Relation {
            name: row.get("name"),
            schema: row.get("schema"),
            partitioned: row.get("partitioned"),
        });
    }
    Ok(children)
}
