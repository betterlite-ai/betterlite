use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture, Row};
use betterlite_lens::models::job::Lens;
use betterlite_redis::request_batch::lock;

#[derive(Clone, Default)]
pub(crate) struct Store(pub(crate) Arc<Mutex<Option<Lens>>>);

pub(crate) struct FakeDatabase(pub(crate) Store);

impl Database for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        Box::pin(ready(Ok(rows(&self.0, query, args))))
    }

    fn execute_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(ready(Ok(0)))
    }
}

fn text_argument(args: &[SqlBind], index: usize) -> Option<&str> {
    match args.get(index) {
        Some(SqlBind::Text(Some(value))) => Some(value),
        _ => None,
    }
}

fn integer_argument(args: &[SqlBind], index: usize) -> Option<i64> {
    match args.get(index) {
        Some(SqlBind::Integer(Some(value))) => Some(*value),
        _ => None,
    }
}

fn rows(store: &Store, query: &str, args: &[SqlBind]) -> Vec<Row> {
    let mut guard = lock(&store.0);
    if query.starts_with("WITH previous") {
        let version = integer_argument(args, 2);
        let Some(previous) = guard.as_ref() else {
            return count("0");
        };
        if Some(previous.id.as_str()) != text_argument(args, 1) || Some(previous.version) != version
        {
            return count("0");
        }
        let Some(data) = text_argument(args, 0) else {
            return count("0");
        };
        let Ok(updated) = serde_json::from_str::<Lens>(data) else {
            return count("0");
        };
        *guard = Some(updated);
        return count("1");
    }
    match guard.as_ref() {
        Some(lens) if Some(lens.id.as_str()) == text_argument(args, 0) => {
            match serde_json::to_string(lens) {
                Ok(data) => vec![Row { data }],
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

fn count(value: &str) -> Vec<Row> {
    vec![Row {
        data: value.to_owned(),
    }]
}
