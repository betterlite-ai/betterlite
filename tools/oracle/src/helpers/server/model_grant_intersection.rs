mod fixture;

use betterlite_auth::model_grant_intersection::collect::{MatchedRequest, collect_matched_groups};
use betterlite_auth::model_grant_intersection::enforced::{
    EnforcedRequest, enforced_model_allowlists,
};
use betterlite_auth::model_grant_intersection::error::IntersectionError;
use betterlite_auth::model_grant_intersection::sources::GrantSources;
use betterlite_auth::model_grant_intersection::stamp::{
    StampRequest, matched_model_access_groups, stamp_matched_groups,
};
use betterlite_auth::model_grant_intersection::token::GrantToken;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixture::{Fixture, Function, Input};

// the project lookup's clock only stamps the cache entry; nothing in the walks compares it
const NOW: f64 = 0.0;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::build(input)?;
        let auth = fixture::auth(input)?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let outcome = match input.function {
            Function::Enforced => runtime.block_on(enforced(input, &fixture, &auth)),
            Function::Collected => runtime.block_on(collected(input, &fixture, &auth)),
            Function::Stamped => Ok(runtime.block_on(stamped(input, &fixture, auth))),
        };
        let (value, error) = split(outcome);
        Ok(json!({"value": value, "error": error, "events": fixture.events()}))
    }
}

type Answer = Result<Value, IntersectionError>;

fn split(answer: Answer) -> (Value, Value) {
    match answer {
        Ok(value) => (value, Value::Null),
        Err(error) => (Value::Null, json!({"status": error.http_status()})),
    }
}

async fn enforced(input: &Input, fixture: &Fixture, auth: &UserAPIKeyAuth) -> Answer {
    let token = GrantToken::of(auth);
    let sources: Option<&dyn GrantSources> = if input.database { Some(fixture) } else { None };
    let allowlists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources,
        now: NOW,
    })
    .await?;
    Ok(json!(allowlists))
}

async fn collected(input: &Input, fixture: &Fixture, auth: &UserAPIKeyAuth) -> Answer {
    let token = GrantToken::of(auth);
    let sources: Option<&dyn GrantSources> = if input.database { Some(fixture) } else { None };
    let matched = collect_matched_groups(MatchedRequest {
        models: input.models.as_deref(),
        token: Some(&token),
        team: fixture.team.as_ref(),
        project: fixture.project.as_ref(),
        membership: fixture.membership.clone(),
        membership_loaded: input.membership_loaded,
        groups: Some(fixture),
        sources,
    })
    .await?;
    Ok(json!(matched))
}

async fn stamped(input: &Input, fixture: &Fixture, mut auth: UserAPIKeyAuth) -> Value {
    let sources: Option<&dyn GrantSources> = if input.database { Some(fixture) } else { None };
    let returned = stamp_matched_groups(StampRequest {
        models: input.models.as_deref(),
        auth: Some(&mut auth),
        team: fixture.team.as_ref(),
        project: fixture.project.as_ref(),
        membership: fixture.membership.clone(),
        membership_loaded: input.membership_loaded,
        groups: Some(fixture),
        sources,
    })
    .await;
    let stored = matched_model_access_groups(&auth);
    json!({"returned": returned, "stored": stored})
}
