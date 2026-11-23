use betterlite_auth::auth_end_users::defaults::{
    DefaultBudgetLookup, MemberBudgetError, key_end_user_budget_id,
};
use betterlite_auth::auth_end_users::object::{EndUserObjectError, EndUserObjects, EndUserRequest};
use betterlite_auth::end_user_registry::EndUserRegistry;
use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixture::{Fixture, budget_view, end_user_view};

mod fixture;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    KeyBudgetId,
    DefaultBudget,
    TeamMemberBudget,
    ResolveDefault,
    EndUserObject,
}

// the one failure a case scripts; every fake row read and write failure is one of these
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    Query,
    Decode,
    EndUserQuery,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    database: bool,
    validate_in_db: bool,
    key_metadata: Option<Payload>,
    proxy_default: Option<String>,
    budget_id: Option<String>,
    key_budget_id: Option<String>,
    cached_budget: Option<LiteLLMBudgetTable>,
    budget_row: Option<LiteLLMBudgetTable>,
    fail: Option<Failure>,
    end_user_id: Option<String>,
    cached_end_user: Option<LiteLLMEndUserTable>,
    end_user_row: Option<LiteLLMEndUserTable>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::new(input).or_value_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(answer(input, &fixture))
    }
}

async fn answer(input: &Input, fixture: &Fixture) -> HelperResult {
    let lookup = DefaultBudgetLookup {
        rows: input.database.then_some(fixture),
        cache: fixture,
        max_end_user_budget_id: input.proxy_default.as_deref(),
    };
    let (value, raised) = selected(input, fixture, &lookup).await?;
    let stored = fixture.stored_end_user(input.end_user_id.as_deref())?;
    let effects = fixture.tracked().clone();
    Ok(json!({
        "value": value,
        "stored": stored,
        "raised": raised,
        "reads": effects.reads,
        "writes": effects.writes,
        "queries": effects.queries,
    }))
}

async fn selected(
    input: &Input,
    fixture: &Fixture,
    lookup: &DefaultBudgetLookup<'_, Fixture, Fixture>,
) -> Result<(Value, Option<&'static str>), Box<HelperError>> {
    match &input.function {
        Function::KeyBudgetId => Ok((
            json!(key_end_user_budget_id(input.key_metadata.as_ref())),
            None,
        )),
        Function::DefaultBudget => {
            let budget = lookup
                .default_end_user_budget(input.budget_id.as_deref())
                .await
                .or_value_error()?;
            Ok((budget_view(budget.as_ref())?, None))
        }
        Function::TeamMemberBudget => {
            let member = lookup
                .team_member_default_budget(input.budget_id.as_deref().unwrap_or_default())
                .await;
            Ok(match member {
                Ok(budget) => (budget_view(budget.as_ref())?, None),
                Err(MemberBudgetError::Decode(_)) => (Value::Null, Some("ValidationError")),
                Err(MemberBudgetError::Cache(_)) => (Value::Null, Some("RuntimeError")),
            })
        }
        Function::ResolveDefault => {
            let budget = lookup
                .resolve_default_budget(input.key_budget_id.as_deref())
                .await
                .or_value_error()?;
            Ok((budget_view(budget.as_ref())?, None))
        }
        Function::EndUserObject => end_user_object(input, fixture).await,
    }
}

async fn end_user_object(
    input: &Input,
    fixture: &Fixture,
) -> Result<(Value, Option<&'static str>), Box<HelperError>> {
    let registry = EndUserRegistry::new(Settings {
        validate_end_user_id_in_db: input.validate_in_db,
        max_end_user_budget_id: input.proxy_default.clone(),
    });
    let store = PostgresStore { pool: None };
    let objects = EndUserObjects {
        rows: input.database.then_some(fixture),
        budgets: input.database.then_some(fixture),
        cache: fixture,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: input.proxy_default.as_deref(),
    };
    let request = EndUserRequest {
        end_user_id: input.end_user_id.as_deref(),
        key_end_user_budget_id: input.key_budget_id.as_deref(),
        token_end_user_max_budget: None,
    };
    match objects.get_end_user_object(&request).await {
        Ok(Some(found)) => Ok((end_user_view(&found)?, None)),
        Ok(None) => Ok((Value::Null, None)),
        Err(EndUserObjectError::NoDatabase) => Ok((Value::Null, Some("Exception"))),
        Err(
            EndUserObjectError::CacheRead(_)
            | EndUserObjectError::Registry(_)
            | EndUserObjectError::Defaults(_),
        ) => Ok((Value::Null, Some("RuntimeError"))),
    }
}
