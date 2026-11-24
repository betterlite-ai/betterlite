use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::finding::Finding;
use betterlite_lens::models::job::Lens;
use betterlite_lens::models::messages::FindingUpdate;
use betterlite_lens::sources::storage::Storage;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_access::state::LensRoutesState;
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{LensRouteError, update_failure};

pub(crate) async fn update_finding<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path((lens_id, finding_id)): Path<(String, String)>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let update: FindingUpdate = decode(&body)?;
    let scope = user_scope(&caller_key(&context.principal)?, true)?;
    let repository = lens_repository(&state)?;
    get_lens(repository, &lens_id, &scope).await?;
    let updated = repository
        .update(
            &lens_id,
            |lens| Ok(patched(lens, &finding_id, &update)),
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(update_failure)?;
    Ok(Json(required(updated)?))
}

fn patched(lens: Lens, finding_id: &str, update: &FindingUpdate) -> Lens {
    let findings = lens
        .findings
        .iter()
        .map(|finding| {
            if finding.id == finding_id {
                Finding {
                    status: update.status,
                    reason: update.reason.clone(),
                    ..finding.clone()
                }
            } else {
                finding.clone()
            }
        })
        .collect();
    Lens { findings, ..lens }
}

mod tests;
