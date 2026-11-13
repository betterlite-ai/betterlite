use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::Row;
use serde_json::Value;

use super::{StoredLens, StoredRun, displaced_jobs, integer_argument, set_version, text_argument};

enum Kind {
    Update,
    Worker,
    Run,
    Lens,
}

impl Store {
    pub(super) fn query(&mut self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        match query_kind(query) {
            Kind::Update => self.update_lens(args),
            Kind::Worker => self.worker_query(query, args),
            Kind::Run => self.run_query(query, args),
            Kind::Lens => self.lens_query(query, args),
        }
    }

    fn lens_query(&mut self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        if query.contains("ORDER BY id") {
            return self.lens_rows(None);
        }
        self.lens_rows(text_argument(args, 0))
    }

    fn lens_rows(&self, lens_id: Option<&str>) -> Vec<Row> {
        self.lenses
            .iter()
            .filter(|(id, _)| lens_id.is_none_or(|wanted| *id == wanted))
            .map(|(_, row)| row_of(&row.data))
            .collect()
    }

    fn run_query(&mut self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        let Some(lens_id) = text_argument(args, 0) else {
            return Vec::new();
        };
        if query.contains("LIMIT 50") {
            return self.jobs_page(lens_id, integer_argument(args, 1).unwrap_or_default());
        }
        self.single_job(lens_id, text_argument(args, 1).unwrap_or_default())
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
        let skip = usize::try_from(offset).map_or(entries.len(), |value| value);
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

    fn worker_query(&mut self, query: &str, args: &[SqlBind]) -> Vec<Row> {
        if query.contains("RETURNING") {
            return self.bill_worker(args);
        }
        if query.contains("token_hash=$1") {
            return self.workers_by_hash(text_argument(args, 0));
        }
        self.worker_rows()
    }

    fn worker_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for row in self.workers.values() {
            rows.push(row_of(&row.data));
        }
        rows
    }

    fn workers_by_hash(&self, token_hash: Option<&str>) -> Vec<Row> {
        self.workers
            .values()
            .filter(|row| Some(row.token_hash.as_str()) == token_hash)
            .map(|row| row_of(&row.data))
            .collect()
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
        if let Value::Object(fields) = &mut row.data {
            fields.insert(
                "analysis_key_id".to_owned(),
                Value::String(key_id.to_owned()),
            );
        }
        Vec::from([row_of(&row.data)])
    }

    fn update_lens(&mut self, args: &[SqlBind]) -> Vec<Row> {
        let (Some(data), Some(lens_id), Some(version)) = (
            text_argument(args, 0),
            text_argument(args, 1),
            integer_argument(args, 2),
        ) else {
            return Vec::from([Row {
                data: "0".to_owned(),
            }]);
        };
        if self.conflicts > 0 {
            self.conflicts = self.conflicts.saturating_sub(1);
            if let Some(row) = self.lenses.get_mut(lens_id) {
                row.version = row.version.saturating_add(1);
                set_version(&mut row.data, row.version);
            }
        }
        let Some(row) = self.lenses.get(lens_id) else {
            return Vec::from([Row {
                data: "0".to_owned(),
            }]);
        };
        if row.version != version {
            return Vec::from([Row {
                data: "0".to_owned(),
            }]);
        }
        let Ok(new_data) = serde_json::from_str::<Value>(data) else {
            return Vec::from([Row {
                data: "0".to_owned(),
            }]);
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
            self.runs.entry(job_id).or_insert(StoredRun {
                lens_id: lens_id.to_owned(),
                data: job,
            });
        }
        Vec::from([Row {
            data: "1".to_owned(),
        }])
    }
}

fn query_kind(query: &str) -> Kind {
    if query.starts_with("WITH previous") {
        return Kind::Update;
    }
    if query.contains("LiteLLM_LensWorker") {
        return Kind::Worker;
    }
    if query.contains("LiteLLM_LensRun") {
        return Kind::Run;
    }
    Kind::Lens
}

fn row_of(data: &Value) -> Row {
    let text = data.to_string();
    let row = Row { data: text };
    row
}
