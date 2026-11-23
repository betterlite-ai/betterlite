use std::sync::Mutex;

use betterlite_core::api::skills::create::call::{CreateSkillCall, SkillCreation};
use betterlite_core::api::skills::create::error::{CreateSkillError, failure as create_failure};
use betterlite_core::api::skills::create::service::CreateSkillService;
use betterlite_core::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::skill_calls::convert;
use crate::skill_calls::error::{SkillFailure, no_database_failure};
use crate::skill_calls::session::logging;
use crate::skill_calls::session::{SkillCore, SkillOutcome};
use crate::skill_calls::transport::create::CreateTransport;

// One create call: the provider check and the pre-call logging, then the service that assembles the body.
pub(super) async fn skill(
    session: &SkillCore,
    args: IndexMap<String, Json>,
    blobs: Vec<Vec<u8>>,
) -> Result<SkillOutcome, SkillFailure> {
    let arguments = convert::create::arguments(args)?;
    let call = CreateSkillCall {
        params: arguments.params,
        display_title: arguments.display_title.as_deref(),
        files: arguments.files.as_deref(),
        extra_body: arguments.extra_body.as_ref(),
        kwargs: &arguments.kwargs,
        litellm_call_id: arguments.litellm_call_id.as_deref(),
    };
    if call.params.provider() == LITELLM_PROXY_PROVIDER {
        return Err(no_database_failure(&session.redactor));
    }
    let mut logging = logging::state(
        &session.redactor,
        "create_skill",
        &arguments.kwargs,
        arguments.litellm_call_id.as_deref(),
    )
    .map_err(|error| SkillFailure {
        class: create_failure(CreateSkillError::from(error), &call, &session.redactor).class,
    })?;
    creating(session, &call, &mut logging, blobs).await
}

// The transport holding this call's blobs, then the service that resolves them into the request body.
async fn creating(
    session: &SkillCore,
    call: &CreateSkillCall<'_>,
    logging: &mut LoggingState,
    blobs: Vec<Vec<u8>>,
) -> Result<SkillOutcome, SkillFailure> {
    let transport = CreateTransport {
        client: session.client.clone(),
        blobs: Mutex::new(blobs),
    };
    let service = CreateSkillService {
        http: &transport,
        hosted: &session.hosted,
        anthropic: &session.anthropic,
        core: &session.core,
        redactor: &session.redactor,
    };
    service
        .create_skill(SkillCreation { call, logging })
        .await
        .map(SkillOutcome::Skill)
        .map_err(|failure| SkillFailure {
            class: failure.class,
        })
}
