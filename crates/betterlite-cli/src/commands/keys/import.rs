use std::io::Write;

use chrono::NaiveDateTime;
use http::Method;

use super::api::ApiError;
use super::client::{JsonReply, KeysClient};
use super::dates;
use super::output;
use super::wire::{KeyGenerateBody, SourceKey, SourcePage};
use super::{ImportKeys, KeysError, KeysPorts};

const LIST_PATH: &str = "/key/list";
const GENERATE_PATH: &str = "/key/generate";
const PAGE_SIZE: usize = 100;
const MISSING_ALIAS: &str = "N/A";

struct CreatedSince<'run> {
    at: NaiveDateTime,
    text: &'run str,
}

struct Import<'run> {
    options: &'run ImportKeys,
    since: Option<CreatedSince<'run>>,
    source: KeysClient<'run>,
    destination: KeysClient<'run>,
    stdout: &'run mut dyn Write,
    stderr: &'run mut dyn Write,
}

pub(super) fn import_keys(options: &ImportKeys, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    let KeysPorts {
        context,
        client,
        redactor,
        stdout,
        stderr,
    } = ports;
    let since = parse_filter(options, stderr)?;
    let mut run = Import {
        options,
        since,
        source: KeysClient {
            transport: client,
            base_url: options.source_base_url.trim_end_matches('/'),
            api_key: options.source_api_key.as_ref(),
            redactor,
        },
        destination: KeysClient {
            transport: client,
            base_url: context.base_url.trim_end_matches('/'),
            api_key: context.api_key.as_ref(),
            redactor,
        },
        stdout,
        stderr,
    };
    let outcome = run.run();
    report_outcome(outcome, run.stdout, run.stderr)
}

fn parse_filter<'run>(
    options: &'run ImportKeys,
    stderr: &mut dyn Write,
) -> Result<Option<CreatedSince<'run>>, KeysError> {
    let Some(text) = options.created_since.as_deref() else {
        return Ok(None);
    };
    match dates::parse_created_since(text) {
        Ok(at) => Ok(Some(CreatedSince { at, text })),
        Err(_source) => {
            let _wrote = writeln!(
                stderr,
                "Error: Invalid date format '{text}'. Use YYYY-MM-DD_HH:MM or YYYY-MM-DD"
            );
            Err(KeysError::Aborted)
        }
    }
}

fn report_outcome(
    outcome: Result<(), KeysError>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), KeysError> {
    match outcome {
        Ok(()) => Ok(()),
        Err(KeysError::Client(ApiError::Http { status, body, .. })) => {
            output::print_http_error(status, &body, stdout, stderr).map_err(KeysError::Json)?;
            Err(KeysError::Aborted)
        }
        Err(error) => {
            let _wrote = writeln!(stderr, "Error: {error}");
            Err(KeysError::Aborted)
        }
    }
}

impl Import<'_> {
    fn run(&mut self) -> Result<(), KeysError> {
        let _fetching = writeln!(
            self.stdout,
            "Fetching keys from source server: {}",
            self.options.source_base_url
        );
        let mut keys = self.fetch_pages()?;
        if let Some(since) = &self.since {
            let before = keys.len();
            keys = filter_keys(keys, since.at)?;
            let after = keys.len();
            let _filtered = writeln!(
                self.stdout,
                "Filtered {before} keys to {after} keys created since {}",
                since.text
            );
        }
        if keys.is_empty() {
            let _none = writeln!(self.stdout, "No keys found in source instance.");
            return Ok(());
        }
        let found = keys.len();
        let _found = writeln!(self.stdout, "Found {found} keys in source instance.");
        if self.options.dry_run {
            output::write_dry_run_table(&keys, self.stdout)?;
            return Ok(());
        }
        let (imported, failed) = self.import_each(keys);
        let _completed = writeln!(self.stdout, "\nImport completed:");
        let _imported = writeln!(self.stdout, "  Successfully imported: {imported}");
        let _failed = writeln!(self.stdout, "  Failed to import: {failed}");
        let _total = writeln!(self.stdout, "  Total keys processed: {found}");
        Ok(())
    }

    fn fetch_pages(&mut self) -> Result<Vec<SourceKey>, KeysError> {
        let mut keys = Vec::new();
        let mut page = 1_u64;
        loop {
            let query = page_query(page);
            let reply = self.source.send(Method::GET, LIST_PATH, &query, None)?;
            if !reply.value.is_object() {
                return Err(KeysError::PageNotAnObject { page });
            }
            let parsed: SourcePage =
                serde_path_to_error::deserialize(reply.value).map_err(|error| {
                    KeysError::PageField {
                        page,
                        field: error.path().to_string(),
                        source: error.into_inner(),
                    }
                })?;
            let page_keys = parsed.keys;
            if page_keys.is_empty() {
                break;
            }
            let count = page_keys.len();
            keys.extend(page_keys);
            let _fetched = writeln!(self.stdout, "Fetched page {page}: {count} keys");
            if count < PAGE_SIZE {
                break;
            }
            page = page.checked_add(1).ok_or(KeysError::PageOverflow)?;
        }
        Ok(keys)
    }

    fn import_each(&mut self, keys: Vec<SourceKey>) -> (usize, usize) {
        let mut imported = 0_usize;
        let mut failed = 0_usize;
        for key in keys {
            let alias = key
                .key_alias
                .clone()
                .unwrap_or_else(|| String::from(MISSING_ALIAS));
            let outcome = self.import_one(key);
            match outcome {
                Ok(reply) => {
                    let _generated =
                        output::write_json(&reply.value, output::JsonText::Generated, self.stdout);
                    imported = imported.saturating_add(1);
                    let _imported = writeln!(self.stdout, "Imported key: {alias}");
                }
                Err(error) => {
                    failed = failed.saturating_add(1);
                    let _failed = writeln!(self.stderr, "Failed to import key {alias}: {error}");
                }
            }
        }
        (imported, failed)
    }

    fn import_one(&self, key: SourceKey) -> Result<JsonReply, KeysError> {
        let body = serde_json::to_vec(&destination_body(key)).map_err(KeysError::Json)?;
        self.destination
            .send(Method::POST, GENERATE_PATH, &[], Some(body))
            .map_err(KeysError::Client)
    }
}

fn page_query(page: u64) -> Vec<(String, String)> {
    vec![
        (String::from("page"), page.to_string()),
        (String::from("size"), PAGE_SIZE.to_string()),
        (String::from("return_full_object"), String::from("true")),
    ]
}

fn filter_keys(keys: Vec<SourceKey>, since: NaiveDateTime) -> Result<Vec<SourceKey>, KeysError> {
    let mut kept = Vec::new();
    for key in keys {
        let Some(created_at) = key.created_at.as_deref() else {
            continue;
        };
        let created =
            dates::parse_created_at(created_at).map_err(|source| KeysError::CreatedAt {
                value: created_at.to_owned(),
                source,
            })?;
        if created >= since {
            kept.push(key);
        }
    }
    Ok(kept)
}

fn destination_body(key: SourceKey) -> KeyGenerateBody {
    KeyGenerateBody {
        models: key.models,
        aliases: key.aliases,
        spend: key.spend,
        duration: None,
        key_alias: key.key_alias,
        team_id: key.team_id,
        user_id: key.user_id,
        budget_id: key.budget_id,
        config: key.config,
    }
}
