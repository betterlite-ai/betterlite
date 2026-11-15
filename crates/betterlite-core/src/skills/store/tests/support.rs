use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_identity_history::SCHEMA_UPDATE;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tracing::span::{Attributes, Id, Record};
use tracing::{Dispatch, Event, Level, Metadata, Subscriber};

use crate::skills::store::SkillPage;
use crate::skills::store::repository::{SkillRepository, SkillWrite};

pub(crate) fn principal(user: &str) -> betterlite_auth::auth_identity::records::Principal {
    let document = format!(
        r#"{{"principal_type":"human","subject":"{user}","auth_method":"api_key","user":{{"id":"{user}"}},"teams":[{{"id":"team-1"}}],"organization":{{"id":"org-1"}},"credential_ref":{{"key_id":"key-1","token_id":"token-1"}}}}"#
    );
    serde_json::from_str(&document).unwrap()
}

pub(crate) fn admin(user: &str) -> betterlite_auth::auth_identity::records::Principal {
    let mut identity = principal(user);
    identity.roles = vec![betterlite_auth::auth_identity::roles::Role::PlatformAdmin];
    identity
}

pub(crate) const NOW: &str = "2026-01-01T00:00:00.123456";
pub(crate) const SKILL_ID: &str = "litellm_skill_00000000-0000-4000-8000-000000000000";

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(crate) fn fixed_ids() -> Arc<dyn Fn() -> String + Send + Sync> {
    Arc::new(|| "00000000-0000-4000-8000-000000000000".to_owned())
}

pub(crate) fn fixed_clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 1000.0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Create(SkillWrite),
    FindUnique(String),
    FindMany {
        page: SkillPage,
        created_by: Option<Vec<String>>,
    },
    Delete(String),
}

pub(crate) struct State {
    pub(crate) available: bool,
    pub(crate) effects: Vec<Effect>,
    pub(crate) rows: BTreeMap<String, SkillTable>,
    pub(crate) page: Vec<SkillTable>,
    pub(crate) create_failure: Option<&'static str>,
    pub(crate) find_failure: Option<&'static str>,
    pub(crate) delete_failure: Option<&'static str>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            available: true,
            effects: Vec::new(),
            rows: BTreeMap::new(),
            page: Vec::new(),
            create_failure: None,
            find_failure: None,
            delete_failure: None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ScriptedRepository {
    pub(crate) state: Arc<Mutex<State>>,
}

impl ScriptedRepository {
    pub(crate) fn with_row(skill: SkillTable) -> Self {
        let repository = Self::default();
        repository.update(|state| {
            state.rows.insert(skill.skill_id.clone(), skill);
        });
        repository
    }

    pub(crate) fn update<Change>(&self, change: Change)
    where
        Change: FnOnce(&mut State),
    {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
    }

    pub(crate) fn effects(&self) -> Vec<Effect> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .clone()
    }

    fn failing(&self, failure: Option<&'static str>) -> Result<(), TableError> {
        match failure {
            Some(message) => Err(TableError::Publish {
                message: message.to_owned(),
            }),
            None => Ok(()),
        }
    }
}

pub(crate) fn row_for(write: &SkillWrite, now: &str) -> SkillTable {
    let metadata: BTreeMap<String, JsonData> = write
        .metadata
        .as_ref()
        .map_or_else(BTreeMap::new, |text| serde_json::from_str(text).unwrap());
    SkillTable {
        skill_id: write.skill_id.clone(),
        display_title: write.display_title.clone(),
        description: write.description.clone(),
        instructions: write.instructions.clone(),
        source: write.source.clone(),
        latest_version: None,
        file_content: write.file_content.clone(),
        file_name: write.file_name.clone(),
        file_type: write.file_type.clone(),
        metadata: Some(metadata),
        created_at: Some(timestamp(now)),
        created_by: Some(write.created_by.clone()),
        updated_at: Some(timestamp(now)),
        updated_by: Some(write.updated_by.clone()),
    }
}

fn timestamp(text: &str) -> RowTimestamp {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
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

    async fn create(&self, write: SkillWrite) -> Result<SkillTable, TableError> {
        let failure = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(Effect::Create(write.clone()));
            state.create_failure
        };
        self.failing(failure)?;
        let row = row_for(&write, NOW);
        self.update(|state| {
            state.rows.insert(write.skill_id.clone(), row.clone());
        });
        Ok(row)
    }

    async fn find_unique(&self, skill_id: &str) -> Result<Option<SkillTable>, TableError> {
        let (found, failure) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(Effect::FindUnique(skill_id.to_owned()));
            (state.rows.get(skill_id).cloned(), state.find_failure)
        };
        self.failing(failure)?;
        Ok(found)
    }

    async fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> Result<Vec<SkillTable>, TableError> {
        let (rows, failure) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(Effect::FindMany { page, created_by });
            (state.page.clone(), state.find_failure)
        };
        self.failing(failure)?;
        Ok(rows)
    }

    async fn delete(&self, skill_id: &str) -> Result<(), TableError> {
        let failure = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(Effect::Delete(skill_id.to_owned()));
            state.delete_failure
        };
        self.failing(failure)?;
        self.update(|state| {
            state.rows.remove(skill_id);
        });
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Subscriber for Warnings {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() == &Level::WARN
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().level() == &Level::WARN {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("warn".to_owned());
        }
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

pub(crate) fn capture_warnings<T, Run>(run: Run) -> (T, Vec<String>)
where
    Run: FnOnce() -> T,
{
    let warnings = Warnings::default();
    let subscriber = warnings.clone();
    let value = tracing::subscriber::with_default(Dispatch::new(subscriber), run);
    (value, warnings.messages())
}

pub(crate) fn stored_row(owner: &str) -> SkillTable {
    let write = SkillWrite {
        skill_id: SKILL_ID.to_owned(),
        display_title: Some("Stored".to_owned()),
        description: None,
        instructions: None,
        source: "custom".to_owned(),
        metadata: None,
        file_content: None,
        file_name: None,
        file_type: None,
        created_by: owner.to_owned(),
        updated_by: owner.to_owned(),
    };
    row_for(&write, NOW)
}

pub(crate) fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["skills-store"])
        .url
        .map(SecretString::from)
}

pub(crate) async fn provision() -> PostgresFixture {
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        test_url().as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.resolve().await.unwrap()
}

pub(crate) fn store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

pub(crate) async fn migrate(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .unwrap();
    client
        .batch_execute(from_utf8(SCHEMA_UPDATE.bytes).unwrap())
        .await
        .unwrap();
}
