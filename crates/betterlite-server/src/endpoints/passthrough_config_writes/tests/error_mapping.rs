use axum::http::StatusCode;
use axum::response::IntoResponse as _;

use crate::endpoints::passthrough_config_writes::error::LookupKind;
use crate::endpoints::passthrough_config_writes::error::PassthroughWriteError;

use super::support;

// both lookups compare the same id over the same list, so the route cannot reach the second miss; its text is answered here
#[tokio::test]
async fn missing_index_answers_404_for_update_and_400_for_delete() {
    let update = support::Reply::from_response(
        PassthroughWriteError::MissingIndex {
            endpoint_id: "ep1".to_owned(),
            kind: LookupKind::Update,
        }
        .into_response(),
    )
    .await;
    assert_eq!(update.status, StatusCode::NOT_FOUND);
    assert_eq!(
        update.detail_error(),
        "Could not find index for endpoint with ID 'ep1'"
    );
    let delete = support::Reply::from_response(
        PassthroughWriteError::MissingIndex {
            endpoint_id: "ep1".to_owned(),
            kind: LookupKind::Delete,
        }
        .into_response(),
    )
    .await;
    assert_eq!(delete.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        delete.detail_error(),
        "Could not find index for endpoint with ID 'ep1'"
    );
}
