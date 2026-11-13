use betterlite_db::daily_spend_sql::bind::SqlBind;
use serde_json::Value;

use super::{Store, StoredLens, StoredWorker, text_argument};

impl Store {
    pub(super) fn execute(&mut self, query: &str, args: &[SqlBind]) -> u64 {
        if query.starts_with(r#"INSERT INTO "LiteLLM_Lens""#) {
            return self.insert_lens(args);
        }
        if query.starts_with(r#"INSERT INTO "LiteLLM_LensWorker""#) {
            return self.insert_worker(args);
        }
        if query.starts_with(r#"UPDATE "LiteLLM_LensWorker" SET data=$1::jsonb"#) {
            return self.update_worker(args);
        }
        if query.contains("{revoked}") {
            return self.revoke_worker(args);
        }
        if query.contains("{last_seen}") {
            return self.heartbeat_worker(args);
        }
        0
    }

    fn insert_lens(&mut self, args: &[SqlBind]) -> u64 {
        let (Some(lens_id), Some(data)) = (text_argument(args, 0), text_argument(args, 1)) else {
            return 0;
        };
        if let Some(data) = row_value(data) {
            self.lenses
                .insert(lens_id.to_owned(), StoredLens { version: 0, data });
        }
        1
    }

    fn insert_worker(&mut self, args: &[SqlBind]) -> u64 {
        let (Some(worker_id), Some(token_hash), Some(data)) = (
            text_argument(args, 0),
            text_argument(args, 1),
            text_argument(args, 2),
        ) else {
            return 0;
        };
        if let Some(data) = row_value(data) {
            self.workers.insert(
                worker_id.to_owned(),
                StoredWorker {
                    token_hash: token_hash.to_owned(),
                    data,
                },
            );
        }
        1
    }

    fn update_worker(&mut self, args: &[SqlBind]) -> u64 {
        let (Some(data), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1)) else {
            return 0;
        };
        if let (Some(data), Some(row)) = (row_value(data), self.workers.get_mut(worker_id)) {
            row.data = data;
        }
        1
    }

    fn revoke_worker(&mut self, args: &[SqlBind]) -> u64 {
        let Some(row) = text_argument(args, 0).and_then(|id| self.workers.get_mut(id)) else {
            return 1;
        };
        if let Value::Object(fields) = &mut row.data {
            fields.insert("revoked".to_owned(), Value::Bool(true));
        }
        1
    }

    fn heartbeat_worker(&mut self, args: &[SqlBind]) -> u64 {
        let (Some(now), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1)) else {
            return 0;
        };
        if let Some(row) = self.workers.get_mut(worker_id)
            && let Value::Object(fields) = &mut row.data
        {
            fields.insert("last_seen".to_owned(), Value::String(now.to_owned()));
        }
        1
    }
}

fn row_value(data: &str) -> Option<Value> {
    serde_json::from_str::<Value>(data)
        .map_err(|error| error.to_string())
        .ok()
}
