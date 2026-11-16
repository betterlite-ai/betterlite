use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{Cursor, Write as _};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::request::Builder as RequestBuilder;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_core::api::stored_skill_reads::StoredSkillReads;
use betterlite_core::api::stored_skill_reads::cache::SkillCacheLimits;
use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::{SkillRepository, SkillWrite};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_http::skill_archive::SkillArchive;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::agent_skills_routes::archives::ArchiveBuilder;
use crate::agent_skills_routes::skills::StoredSkills;
use crate::agent_skills_routes::{AgentSkillsState, routes};
use crate::custom_url::CustomUrls;

pub(super) const STAMP: &str = "2026-01-01T00:00:00.123456";
pub(super) const LATER_STAMP: &str = "2026-02-02T00:00:00";
pub(super) const CLOCK_START: f64 = 1_000.0;
pub(super) const BOGUS_KEY: &str = "Bearer not-a-configured-key";
pub(super) const REAL_MANIFEST: &[u8] =
    b"---\nname:  Demo skill \ndescription: ' A demo ' \n---\nbody";
// the stored-skill reader has its own row cache; it stays off so a rewritten row is read on the next request
const NO_ROW_CACHE: SkillCacheLimits = SkillCacheLimits {
    max_size_in_memory: 0,
    default_ttl: 0.0,
};

#[derive(Deserialize)]
pub(super) struct IndexWire {
    #[serde(rename = "$schema")]
    pub(super) discovery_schema: String,
    pub(super) skills: Vec<EntryWire>,
}

#[derive(Deserialize)]
pub(super) struct EntryWire {
    pub(super) name: String,
    #[serde(rename = "type")]
    pub(super) entry_type: String,
    pub(super) description: String,
    pub(super) url: String,
    pub(super) digest: String,
}

#[derive(Deserialize)]
pub(super) struct DetailWire {
    pub(super) detail: String,
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn header(&self, name: header::HeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub(super) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(super) struct ManualClock {
    seconds: Mutex<f64>,
}

fn clock_handle(clock: &Arc<ManualClock>) -> Clock {
    let reading = Arc::clone(clock);
    Arc::new(move || {
        *reading
            .seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    })
}

impl ManualClock {
    pub(super) fn advance_to(&self, seconds: f64) {
        *self.seconds.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
    }
}

fn timestamp(text: &str) -> RowTimestamp {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
}

pub(super) fn row(skill_id: &str, upload: Option<&[u8]>, updated_at: Option<&str>) -> SkillTable {
    SkillTable {
        skill_id: skill_id.to_owned(),
        display_title: None,
        description: None,
        instructions: None,
        source: "custom".to_owned(),
        latest_version: None,
        file_content: upload.map(|bytes| STANDARD.encode(bytes)),
        file_name: None,
        file_type: None,
        metadata: None,
        created_at: None,
        created_by: None,
        updated_at: updated_at.map(timestamp),
        updated_by: None,
    }
}

pub(super) fn skill_archive(
    name: Option<&str>,
    description: Option<&str>,
    digest: &str,
) -> SkillArchive {
    SkillArchive {
        content: b"repacked-zip".to_vec(),
        digest: digest.to_owned(),
        declared_name: name.map(str::to_owned),
        declared_description: description.map(str::to_owned),
    }
}

pub(super) fn sized_archive(len: usize) -> SkillArchive {
    SkillArchive {
        content: vec![7_u8; len],
        digest: "sha256:sized".to_owned(),
        declared_name: None,
        declared_description: None,
    }
}

pub(super) fn zip_upload(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in members {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

pub(super) fn archive_path(skill_id: &str) -> String {
    format!("/v1/skills/{skill_id}/archive")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Call {
    FindUnique(String),
    FindMany {
        page: SkillPage,
        created_by: Option<Vec<String>>,
    },
}

#[derive(Clone, Copy)]
enum RepositoryMode {
    Listing,
    Unavailable,
    Failing,
}

struct RepositoryState {
    available: bool,
    failing: bool,
    rows: Vec<SkillTable>,
    calls: Vec<Call>,
}

fn scripted_failure() -> TableError {
    TableError::Publish {
        message: "the scripted repository failed the read".to_owned(),
    }
}

#[derive(Clone)]
pub(super) struct ScriptedRepository {
    state: Arc<Mutex<RepositoryState>>,
}

impl ScriptedRepository {
    fn scripted(rows: Vec<SkillTable>, mode: RepositoryMode) -> Self {
        Self {
            state: Arc::new(Mutex::new(RepositoryState {
                available: !matches!(mode, RepositoryMode::Unavailable),
                failing: matches!(mode, RepositoryMode::Failing),
                rows,
                calls: Vec::new(),
            })),
        }
    }

    fn listing(rows: Vec<SkillTable>) -> Self {
        Self::scripted(rows, RepositoryMode::Listing)
    }

    pub(super) fn unavailable() -> Self {
        Self::scripted(Vec::new(), RepositoryMode::Unavailable)
    }

    pub(super) fn failing_reads() -> Self {
        Self::scripted(Vec::new(), RepositoryMode::Failing)
    }

    fn record_read(&self, call: Call) -> Result<Vec<SkillTable>, TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.calls.push(call);
        if state.failing {
            return Err(scripted_failure());
        }
        Ok(state.rows.clone())
    }

    pub(super) fn replace_row(&self, replacement: SkillTable) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let found = state
            .rows
            .iter()
            .position(|existing| existing.skill_id == replacement.skill_id);
        match found {
            Some(index) => state.rows[index] = replacement,
            None => state.rows.push(replacement),
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .clone()
    }

    pub(super) fn page_calls(&self) -> Vec<(SkillPage, Option<Vec<String>>)> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::FindMany { page, created_by } => Some((page, created_by)),
                Call::FindUnique(_) => None,
            })
            .collect()
    }

    pub(super) fn find_unique_calls(&self) -> Vec<String> {
        let mut found = Vec::new();
        for call in self.calls() {
            if let Call::FindUnique(skill_id) = call {
                found.push(skill_id);
            }
        }
        found
    }
}

impl SkillRepository for ScriptedRepository {
    fn availability(&self) -> Result<(), TableError> {
        let available = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available;
        if available {
            return Ok(());
        }
        Err(TableError::Database(DbError::NoDbConnected))
    }

    fn create(
        &self,
        _write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(Err(scripted_failure()))
    }

    fn find_unique(
        &self,
        skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        let skill_id = skill_id.to_owned();
        async move {
            let rows = self.record_read(Call::FindUnique(skill_id.clone()))?;
            Ok(rows
                .into_iter()
                .find(|existing| existing.skill_id == skill_id))
        }
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        async move { self.record_read(Call::FindMany { page, created_by }) }
    }

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(scripted_failure()))
    }
}

pub(super) struct ScriptedArchives {
    answers: Mutex<BTreeMap<Vec<u8>, Option<SkillArchive>>>,
    uploads: Mutex<Vec<Vec<u8>>>,
}

impl ScriptedArchives {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(BTreeMap::new()),
            uploads: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn answering(&self, upload: &[u8], archive: SkillArchive) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(upload.to_vec(), Some(archive));
    }

    pub(super) fn uploads(&self) -> Vec<Vec<u8>> {
        self.uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ArchiveBuilder for ScriptedArchives {
    fn build(&self, upload: &[u8]) -> Option<SkillArchive> {
        self.uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(upload.to_vec());
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(upload)
            .cloned()
            .flatten()
    }
}

pub(super) struct Harness {
    pub(super) repository: ScriptedRepository,
    pub(super) archives: Arc<ScriptedArchives>,
    pub(super) clock: Arc<ManualClock>,
    pub(super) custom_urls: CustomUrls,
    index_enabled: bool,
}

impl Harness {
    pub(super) fn with_repository(repository: ScriptedRepository) -> Self {
        Self {
            repository,
            archives: ScriptedArchives::new(),
            clock: Arc::new(ManualClock {
                seconds: Mutex::new(CLOCK_START),
            }),
            custom_urls: CustomUrls::default(),
            index_enabled: true,
        }
    }

    pub(super) fn new(rows: Vec<SkillTable>) -> Self {
        Self::with_repository(ScriptedRepository::listing(rows))
    }

    pub(super) fn disabled(rows: Vec<SkillTable>) -> Self {
        let mut harness = Self::new(rows);
        harness.index_enabled = false;
        harness
    }

    pub(super) fn router(&self) -> Router {
        let archives: Arc<dyn ArchiveBuilder> = Arc::clone(&self.archives);
        self.router_with(archives)
    }

    pub(super) fn router_with(&self, archives: Arc<dyn ArchiveBuilder>) -> Router {
        let skills: Arc<dyn StoredSkills> = Arc::new(
            StoredSkillReads::new(
                self.repository.clone(),
                NO_ROW_CACHE,
                clock_handle(&self.clock),
            )
            .unwrap(),
        );
        let state = AgentSkillsState::new(
            self.index_enabled,
            skills,
            archives,
            self.custom_urls.clone(),
            clock_handle(&self.clock),
        )
        .unwrap();
        routes(state)
    }
}

async fn reply(response: Response) -> Reply {
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

pub(super) async fn send(router: &Router, request: Request<Body>) -> Reply {
    reply(router.clone().oneshot(request).await.unwrap()).await
}

pub(super) fn get_request(path: &str) -> RequestBuilder {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, "proxy.test")
}

pub(super) async fn get(router: &Router, path: &str) -> Reply {
    send(router, get_request(path).body(Body::empty()).unwrap()).await
}

pub(super) fn many_skills(count: usize, archives: &ScriptedArchives) -> Vec<SkillTable> {
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let upload = format!("upload-{index:03}");
        archives.answering(
            upload.as_bytes(),
            skill_archive(None, None, &format!("sha256:{index:03}")),
        );
        rows.push(row(
            &format!("skill-{index:03}"),
            Some(upload.as_bytes()),
            Some(STAMP),
        ));
    }
    rows
}
