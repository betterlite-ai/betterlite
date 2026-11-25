use axum::Json;
use axum::extract::{Extension, Path, RawQuery, State};
use betterlite_db::lens_repository::database::Database;
use betterlite_lens::models::execution::{Execution, ExecutionContent, ExecutionSource};
use betterlite_lens::models::job::Lens;
use betterlite_lens::sources::parse_execution::parse_execution;
use betterlite_lens::sources::storage::Storage;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::lookup::{get_lens, lens_repository};
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_access::state::LensRoutesState;
use crate::lens_access::storage::source_reader;
use crate::lens_admin_routes::body::decode_content_query;
use crate::lens_admin_routes::error::LensRouteError;

pub(crate) async fn evidence_content<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path((lens_id, execution_id)): Path<(String, String)>,
    caller: Option<Extension<AuthContext>>,
    raw: RawQuery,
) -> Result<Json<ExecutionContent>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let query = decode_content_query(raw.0.unwrap_or_default().as_bytes())?;
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    let lens = get_lens(lens_repository(&state)?, &lens_id, &scope).await?;
    let Ok(parsed) = parse_execution(&execution_id) else {
        return Err(LensRouteError::Access(LensAccessError::ExecutionNotFound));
    };
    let execution = named_execution(execution_id, &lens, parsed)?;
    Ok(Json(
        source_reader(state.storage.as_ref())?
            .content(&lens.scope, &execution, &query.cursor, query.offset)
            .await?,
    ))
}

// the execution the activity preview would have listed for this id, once its team is covered
fn named_execution(
    execution_id: String,
    lens: &Lens,
    parsed: (String, String, String, String),
) -> Result<Execution, LensAccessError> {
    let (source_name, team, trace_id, trace_ref) = parsed;
    let (source, root_seen) = match source_name.as_str() {
        "traces" => (ExecutionSource::Traces, false),
        "requests" => (ExecutionSource::Requests, true),
        _ => return Err(LensAccessError::ExecutionNotFound),
    };
    if !lens.scope.all_teams && team != lens.scope.team_id {
        return Err(LensAccessError::ExecutionNotFound);
    }
    Ok(Execution {
        id: execution_id,
        source,
        trace_id: trace_id.clone(),
        trace_ref,
        team_id: team,
        name: trace_id,
        start_time: String::new(),
        span_count: 1,
        root_seen,
        service: String::new(),
        metadata: Vec::new(),
    })
}

mod tests;
