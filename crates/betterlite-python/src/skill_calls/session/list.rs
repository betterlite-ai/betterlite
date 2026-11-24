use betterlite_core::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;
use betterlite_core::api::skills::list::call::{ListSkillsCall, SkillListing};
use betterlite_core::api::skills::list::error::{ListSkillsError, failure as list_failure};
use betterlite_core::api::skills::list::service::ListSkillsService;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::skill_calls::convert;
use crate::skill_calls::error::{SkillFailure, no_database_failure};
use crate::skill_calls::session::call_logging;
use crate::skill_calls::session::{SkillCore, SkillOutcome};

// One list call: the provider check and the pre-call logging, then the service that pages the answer.
pub(super) async fn skill(
    session: &SkillCore,
    args: IndexMap<String, Json>,
) -> Result<SkillOutcome, SkillFailure> {
    let arguments = convert::list::arguments(args)?;
    let call = ListSkillsCall {
        params: arguments.params,
        limit: arguments.limit,
        page: arguments.page.as_deref(),
        source: arguments.source.as_deref(),
        kwargs: &arguments.kwargs,
        litellm_call_id: arguments.litellm_call_id.as_deref(),
    };
    if call.params.provider() == LITELLM_PROXY_PROVIDER {
        return Err(no_database_failure(&session.redactor));
    }
    let mut logging = call_logging(
        &session.redactor,
        "list_skills",
        &arguments.kwargs,
        arguments.litellm_call_id.as_deref(),
    )
    .map_err(|error| SkillFailure {
        class: list_failure(ListSkillsError::from(error), &call, &session.redactor).class,
    })?;
    listing(session, &call, &mut logging).await
}

// The shared read side, then the service that builds the query and pages the answer.
async fn listing(
    session: &SkillCore,
    call: &ListSkillsCall<'_>,
    logging: &mut LoggingState,
) -> Result<SkillOutcome, SkillFailure> {
    let service = ListSkillsService {
        http: &session.http,
        hosted: &session.hosted,
        anthropic: &session.anthropic,
        core: &session.core,
        redactor: &session.redactor,
    };
    service
        .list_skills(SkillListing { call, logging })
        .await
        .map(SkillOutcome::Listed)
        .map_err(|failure| SkillFailure {
            class: failure.class,
        })
}
