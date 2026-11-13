use betterlite_db::daily_spend_sql::bind::SqlBind;
use serde_json::Value;

use super::{StoredLens, StoredWorker, Store, text_argument};

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
        self.patch_worker(query, args)
    }

    fn insert_lens(&mut self, args: &[SqlBind]) -> u64 {
        let (Some(lens_id), Some(data)) = (text_argument(args, 0), text_argument(args, 1)) else {
            return 0;
        };
        if let Ok(data) = serde_json::from_str::<Value>(data) {
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
        if let Ok(data) = serde_json::from_str::<Value>(data) {
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
        let (Some(data), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1))
        else {
            return 0;
        };
        if let Ok(data) = serde_json::from_str::<Value>(data)
            && let Some(row) = self.workers.get_mut(worker_id)
        {
            row.data = data;
        }
        1
    }

    fn patch_worker(&mut self, query: &str, args: &[SqlBind]) -> u64 {
        if query.contains("{revoked}") {
            if let Some(worker_id) = text_argument(args, 0)
                && let Some(row) = self.workers.get_mut(worker_id)
                && let Value::Object(fields) = &mut row.data
            {
                fields.insert("revoked".to_owned(), Value::Bool(true));
            }
            return 1;
        }
        if query.contains("{last_seen}") {
            if let (Some(now), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1))
                && let Some(row) = self.workers.get_mut(worker_id)
                && let Value::Object(fields) = &mut row.data
            {
                fields.insert("last_seen".to_owned(), Value::String(now.to_owned()));
            }
            return 1;
        }
        0
    }
}
