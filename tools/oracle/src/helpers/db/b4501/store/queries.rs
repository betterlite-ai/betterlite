use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::Row;
use serde_json::Value;

use super::{
    Store, StoredLens, StoredRun, StoredWorker, displaced_jobs, integer_argument, text_argument,
};

impl Store {
    pub(super) fn query(&mut self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        if query.starts_with("WITH previous") {
            return self.update_lens(args);
        }
        if query.contains("LiteLLM_LensWorker") {
            return self.bill_worker(args);
        }
        let Some(lens_id) = text_argument(args, 0) else {
            return Vec::new();
        };
        if query.contains("LIMIT 50") {
            return self.jobs_page(lens_id, integer_argument(args, 1).unwrap_or_default());
        }
        if query.contains("LiteLLM_LensRun") {
            return self.single_job(lens_id, text_argument(args, 1).unwrap_or_default());
        }
        self.lenses
            .get(lens_id)
            .map(|row| row_of(&row.data))
            .into_iter()
            .collect()
    }

    pub(super) fn execute(&mut self, query: &str, args: &[SqlBind]) -> u64 {
        let texts: Vec<&str> = (0..args.len())
            .filter_map(|index| text_argument(args, index))
            .collect();
        match *texts.as_slice() {
            [worker_id, token_hash, data] => self.insert_worker(worker_id, token_hash, data),
            [now, worker_id] if query.contains("{last_seen}") => {
                self.patch_worker(worker_id, "last_seen", Value::String(now.to_owned()));
            }
            [data, worker_id] => self.replace_worker(worker_id, data),
            [worker_id] => self.patch_worker(worker_id, "revoked", Value::Bool(true)),
            _ => return 0,
        }
        1
    }

    fn jobs_page(&self, lens_id: &str, offset: i64) -> Vec<Row> {
        let mut entries: Vec<(String, String, Value)> = Vec::new();
        for (job_id, run) in &self.runs {
            if run.lens_id == lens_id {
                let created = run
                    .data
                    .get("created_at")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                entries.push((created, job_id.clone(), run.data.clone()));
            }
        }
        if let Some(row) = self.lenses.get(lens_id)
            && let Some(jobs) = row.data.get("jobs").and_then(Value::as_array)
        {
            for job in jobs {
                let Some(job_id) = job.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let created = job
                    .get("created_at")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                entries.push((created, job_id.to_owned(), job.clone()));
            }
        }
        entries.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let skip = usize::try_from(offset).unwrap_or(usize::MAX);
        entries
            .into_iter()
            .skip(skip)
            .take(50)
            .map(|entry| row_of(&entry.2))
            .collect()
    }

    fn single_job(&self, lens_id: &str, job_id: &str) -> Vec<Row> {
        if let Some(run) = self.runs.get(job_id)
            && run.lens_id == lens_id
        {
            return Vec::from([row_of(&run.data)]);
        }
        if let Some(row) = self.lenses.get(lens_id)
            && let Some(jobs) = row.data.get("jobs").and_then(Value::as_array)
            && let Some(found) = jobs
                .iter()
                .find(|job| job.get("id").and_then(Value::as_str) == Some(job_id))
        {
            return Vec::from([row_of(found)]);
        }
        Vec::new()
    }

    fn bill_worker(&mut self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(key_id), Some(worker_id)) = (text_argument(args, 0), text_argument(args, 1))
        else {
            return Vec::new();
        };
        let Some(row) = self.workers.get_mut(worker_id) else {
            return Vec::new();
        };
        if row.data.get("revoked") == Some(&Value::Bool(true)) {
            return Vec::new();
        }
        patch(
            &mut row.data,
            "analysis_key_id",
            Value::String(key_id.to_owned()),
        );
        Vec::from([row_of(&row.data)])
    }

    fn update_lens(&mut self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(data), Some(lens_id), Some(version)) = (
            text_argument(args, 0),
            text_argument(args, 1),
            integer_argument(args, 2),
        ) else {
            return count_row("0");
        };
        if self.conflicts > 0 {
            self.conflicts = self.conflicts.saturating_sub(1);
            if let Some(row) = self.lenses.get_mut(lens_id) {
                row.version = row.version.saturating_add(1);
                patch(&mut row.data, "version", Value::from(row.version));
            }
        }
        let Some(row) = self.lenses.get(lens_id) else {
            return count_row("0");
        };
        if row.version != version {
            return count_row("0");
        }
        let Ok(new_data) = serde_json::from_str::<Value>(data) else {
            return count_row("0");
        };
        let displaced = displaced_jobs(&row.data, &new_data);
        self.lenses.insert(
            lens_id.to_owned(),
            StoredLens {
                version: version.saturating_add(1),
                data: new_data,
            },
        );
        for (job_id, job) in displaced {
            self.runs.entry(job_id).or_insert_with(|| StoredRun {
                lens_id: lens_id.to_owned(),
                data: job,
            });
        }
        count_row("1")
    }

    fn insert_worker(&mut self, worker_id: &str, token_hash: &str, text: &str) {
        if let Ok(data) = serde_json::from_str::<Value>(text) {
            let stored = StoredWorker {
                token_hash: token_hash.to_owned(),
                data,
            };
            self.workers.insert(worker_id.to_owned(), stored);
        }
    }

    fn replace_worker(&mut self, worker_id: &str, text: &str) {
        let parsed = serde_json::from_str::<Value>(text);
        if let (Ok(data), Some(row)) = (parsed, self.workers.get_mut(worker_id)) {
            row.data = data;
        }
    }

    fn patch_worker(&mut self, worker_id: &str, field: &str, value: Value) {
        if let Some(row) = self.workers.get_mut(worker_id) {
            patch(&mut row.data, field, value);
        }
    }
}

fn count_row(value: &str) -> Vec<Row> {
    let row = Row {
        data: value.to_owned(),
    };
    Vec::from([row])
}

fn row_of(data: &Value) -> Row {
    Row {
        data: data.to_string(),
    }
}

fn patch(data: &mut Value, field: &str, value: Value) {
    if let Value::Object(fields) = data {
        fields.insert(field.to_owned(), value);
    }
}
