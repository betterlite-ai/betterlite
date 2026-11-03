use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;

async fn batch_execute(client: &Client, sql: &str) {
    client.batch_execute(sql).await.unwrap();
}
async fn batch_execute_err(client: &Client, sql: &str) -> tokio_postgres::Error {
    client.batch_execute(sql).await.unwrap_err()
}
async fn exec_params(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    client.execute(sql, params).await.unwrap();
}
async fn exec_params_err(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> tokio_postgres::Error {
    client.execute(sql, params).await.unwrap_err()
}
async fn scalar_text(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> String {
    client.query_one(sql, params).await.unwrap().get(0)
}
async fn scalar_opt_text(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Option<String> {
    client.query_one(sql, params).await.unwrap().get(0)
}
async fn relation_oid(client: &Client, name: &str) -> String {
    scalar_text(
        client,
        "SELECT oid::text FROM pg_class WHERE relname = $1",
        &[&name],
    )
    .await
}
async fn relation_exists(client: &Client, name: &str) -> bool {
    !client
        .query("SELECT 1 FROM pg_class WHERE relname = $1", &[&name])
        .await
        .unwrap()
        .is_empty()
}
struct ColumnInfo {
    data_type: String,
    is_nullable: bool,
    default: Option<String>,
    udt_name: String,
    datetime_precision: Option<i32>,
}
async fn column(client: &Client, table: &str, name: &str) -> ColumnInfo {
    let rows = columns_in(client, table).await;
    let row = rows
        .iter()
        .find(|row| row.get::<_, String>(1) == name)
        .unwrap();
    ColumnInfo {
        data_type: row.get(2),
        is_nullable: row.get::<_, String>(3) == "YES",
        default: row.get(4),
        udt_name: row.get(5),
        datetime_precision: row.get(6),
    }
}
async fn columns_in(client: &Client, table: &str) -> Vec<Row> {
    client
        .query(
            "SELECT ordinal_position, column_name, data_type, is_nullable, column_default, \
             udt_name, datetime_precision \
             FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 \
             ORDER BY ordinal_position",
            &[&table],
        )
        .await
        .unwrap()
}
async fn query_one_row(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Row {
    client.query_one(sql, params).await.unwrap()
}
async fn query_opt_row(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Option<Row> {
    client.query_opt(sql, params).await.unwrap()
}
async fn string_rows(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<String> {
    let rows = client.query(sql, params).await.unwrap();
    let mut values = Vec::with_capacity(rows.len());
    for row in &rows {
        values.push(row.get(0));
    }
    values
}
type ColumnSpec = (
    &'static str,
    &'static str,
    bool,
    Option<&'static str>,
    &'static str,
    Option<i32>,
);

macro_rules! fixed_column_spec {
    ($name:ident, $data_type:literal, $nullable:expr, $default:expr, $udt_name:literal) => {
        fn $name(name: &'static str) -> ColumnSpec {
            (name, $data_type, $nullable, $default, $udt_name, None)
        }
    };
}
macro_rules! default_column_spec {
    ($name:ident, $data_type:literal, $nullable:expr, $udt_name:literal) => {
        fn $name(name: &'static str, default: &'static str) -> ColumnSpec {
            (name, $data_type, $nullable, Some(default), $udt_name, None)
        }
    };
}
async fn assert_column(client: &Client, table: &str, spec: ColumnSpec) {
    let (name, data_type, nullable, default, udt_name, datetime_precision) = spec;
    let info = column(client, table, name).await;
    assert_eq!(info.data_type, data_type, "{table}.{name} type");
    assert_eq!(info.is_nullable, nullable, "{table}.{name} nullable");
    assert_eq!(info.default.as_deref(), default, "{table}.{name} default");
    assert_eq!(info.udt_name, udt_name, "{table}.{name} udt_name");
    assert_eq!(
        info.datetime_precision, datetime_precision,
        "{table}.{name} datetime_precision"
    );
}
async fn assert_columns(client: &Client, table: &str, specs: &[ColumnSpec]) {
    for spec in specs {
        assert_column(client, table, *spec).await;
    }
}
fixed_column_spec!(bool_true, "boolean", false, Some("true"), "bool");
fixed_column_spec!(bool_false, "boolean", false, Some("false"), "bool");
fixed_column_spec!(nullable_text, "text", true, None, "text");
fixed_column_spec!(required_text, "text", false, None, "text");
fixed_column_spec!(nullable_jsonb, "jsonb", true, None, "jsonb");
fixed_column_spec!(
    nullable_empty_jsonb,
    "jsonb",
    true,
    Some("'{}'::jsonb"),
    "jsonb"
);
fixed_column_spec!(int4_zero, "integer", false, Some("0"), "int4");
fixed_column_spec!(int8_zero, "bigint", false, Some("0"), "int8");
fixed_column_spec!(
    float8_zero,
    "double precision",
    false,
    Some("0.0"),
    "float8"
);
default_column_spec!(text_default, "text", false, "text");
default_column_spec!(nullable_text_array, "ARRAY", true, "_text");
fn timestamp3(name: &'static str, default: Option<&'static str>) -> ColumnSpec {
    (
        name,
        "timestamp without time zone",
        false,
        default,
        "timestamp",
        Some(3_i32),
    )
}
async fn assert_exec_params_err_state(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    code: SqlState,
) {
    assert_eq!(
        sql_state(&exec_params_err(client, sql, params).await),
        code.code()
    );
}
async fn catalog_snapshot(client: &Client) -> Vec<String> {
    let mut snapshot = string_rows(
        client,
        "SELECT table_name || ':' || ordinal_position || ':' || column_name || ':' \
         || data_type || ':' || udt_name || ':' || coalesce(datetime_precision::text, '') \
         || ':' || is_nullable || ':' || coalesce(column_default, '') \
         FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        &[],
    )
    .await;
    let mut indexes = string_rows(
        client,
        "SELECT indexname || ':' || indexdef FROM pg_indexes \
         WHERE schemaname = 'public' ORDER BY indexname",
        &[],
    )
    .await;
    snapshot.append(&mut indexes);
    let mut constraints = string_rows(
        client,
        "SELECT conname || ':' || pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE connamespace = 'public'::regnamespace ORDER BY conname",
        &[],
    )
    .await;
    snapshot.append(&mut constraints);
    snapshot
}
async fn assert_rule(client: &Client, conname: &str, update_rule: char, delete_rule: char) {
    let row = query_one_row(
        client,
        "SELECT confupdtype, confdeltype FROM pg_constraint WHERE conname = $1",
        &[&conname],
    )
    .await;
    let update: i8 = row.get(0);
    let delete: i8 = row.get(1);
    assert_eq!(char::from(u8::try_from(update).unwrap()), update_rule);
    assert_eq!(char::from(u8::try_from(delete).unwrap()), delete_rule);
}
fn sql_state(error: &tokio_postgres::Error) -> &str {
    error.code().map_or("", SqlState::code)
}
