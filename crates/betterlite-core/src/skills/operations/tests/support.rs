use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use crate::skills::operations::{HostedSkillOps, SkillTarget, SkillUpload, SkillUploadType};
use crate::skills::store::repository::{SkillRepository, SkillWrite};
use crate::skills::store::{HostedSkillStore, SkillPage};

pub(crate) const NOW: &str = "2026-01-01T00:00:00.123456";
pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const SKILL_ID: &str = "litellm_skill_00000000-0000-4000-8000-000000000000";

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(crate) fn principal(user: &str) -> Principal {
    let document = format!(
        r#"{{"principal_type":"human","subject":"{user}","auth_method":"api_key","user":{{"id":"{user}"}},"teams":[{{"id":"team-1"}}],"organization":{{"id":"org-1"}},"credential_ref":{{"key_id":"key-1","token_id":"token-1"}}}}"#
    );
    serde_json::from_str(&document).unwrap()
}

pub(crate) fn admin(user: &str) -> Principal {
    let mut identity = principal(user);
    identity.roles = vec![Role::PlatformAdmin];
    identity
}

fn fixed_ids() -> Arc<dyn Fn() -> String + Send + Sync> {
    Arc::new(|| "00000000-0000-4000-8000-000000000000".to_owned())
}

fn fixed_clock() -> Clock {
    Arc::new(|| 1_000.0_f64)
}

pub(crate) fn ops(repository: ScriptedRepository) -> HostedSkillOps<ScriptedRepository> {
    let store = HostedSkillStore::new(repository, fixed_ids(), fixed_clock()).unwrap();
    HostedSkillOps { store }
}

pub(crate) fn target<'request>(
    logging: &'request mut LoggingState,
    skill_id: &'request str,
) -> SkillTarget<'request> {
    SkillTarget {
        logging,
        id: skill_id,
        principal: None,
        call_id: None,
    }
}

pub(crate) fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "create_skill".to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .unwrap()
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").unwrap()
}

pub(crate) fn logged(state: &LoggingState, name: &str) -> Json {
    state
        .model_call_details
        .get(name)
        .cloned()
        .unwrap_or(Json::Null)
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(crate) fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Create(Box<SkillWrite>),
    FindUnique(String),
    FindMany {
        page: SkillPage,
        created_by: Option<Vec<String>>,
    },
    Delete(String),
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) effects: Vec<Effect>,
    pub(crate) rows: BTreeMap<String, SkillTable>,
    pub(crate) page: Vec<SkillTable>,
    pub(crate) create_failure: bool,
}

#[derive(Clone, Default)]
pub(crate) struct ScriptedRepository {
    state: Arc<Mutex<State>>,
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

    fn created_row(&self, write: &SkillWrite) -> Result<SkillTable, TableError> {
        let (failure, row) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(Effect::Create(Box::new(write.clone())));
            let row = row_for(write);
            (state.create_failure, row)
        };
        if failure {
            return Err(TableError::Publish {
                message: "the scripted repository failed the create".to_owned(),
            });
        }
        self.update(|state| {
            state.rows.insert(write.skill_id.clone(), row.clone());
        });
        Ok(row)
    }

    fn found_row(&self, skill_id: &str) -> Option<SkillTable> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::FindUnique(skill_id.to_owned()));
        state.rows.get(skill_id).cloned()
    }

    fn found_page(&self, page: SkillPage, created_by: Option<Vec<String>>) -> Vec<SkillTable> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::FindMany { page, created_by });
        state.page.clone()
    }

    fn deleted_row(&self, skill_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::Delete(skill_id.to_owned()));
        state.rows.remove(skill_id);
    }
}

impl SkillRepository for ScriptedRepository {
    fn availability(&self) -> Result<(), TableError> {
        Ok(())
    }

    fn create(
        &self,
        write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(self.created_row(&write))
    }

    fn find_unique(
        &self,
        skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        ready(Ok(self.found_row(skill_id)))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        ready(Ok(self.found_page(page, created_by)))
    }

    fn delete(&self, skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready({
            self.deleted_row(skill_id);
            Ok(())
        })
    }
}

pub(crate) fn upload<'request>(
    file_name: &'request str,
    content: &'request [u8],
    file_type: SkillUploadType<'request>,
) -> SkillUpload<'request> {
    SkillUpload {
        file_name,
        content,
        file_type,
    }
}

pub(crate) fn timestamped(text: &str) -> RowTimestamp {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
}

fn row_for(write: &SkillWrite) -> SkillTable {
    let metadata: Option<BTreeMap<String, JsonData>> = write
        .metadata
        .as_ref()
        .map(|text| serde_json::from_str(text).unwrap());
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
        metadata,
        created_at: Some(timestamped(NOW)),
        created_by: Some(write.created_by.clone()),
        updated_at: Some(timestamped(NOW)),
        updated_by: Some(write.updated_by.clone()),
    }
}

pub(crate) fn stored_row(owner: &str) -> SkillTable {
    let write = SkillWrite {
        skill_id: SKILL_ID.to_owned(),
        display_title: Some("Stored".to_owned()),
        description: Some("A stored skill".to_owned()),
        instructions: None,
        source: "custom".to_owned(),
        metadata: None,
        file_content: None,
        file_name: None,
        file_type: None,
        created_by: owner.to_owned(),
        updated_by: owner.to_owned(),
    };
    row_for(&write)
}
