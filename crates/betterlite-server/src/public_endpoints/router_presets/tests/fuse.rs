use axum::http::StatusCode;
use betterlite_router::fuse_presets::{FusePresetCatalog, get_fuse_presets};

use super::support::{PRESETS_URL, Script, app, get, settings};
use crate::public_endpoints::router_presets::FUSE_PRESETS_PATH;

#[tokio::test]
async fn serves_the_bundled_fuse_catalog() {
    let script = Script::default();
    let router = app(&script, settings(PRESETS_URL, false));
    let (status, content_type, body) = get(router, FUSE_PRESETS_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    let served: FusePresetCatalog = serde_json::from_str(&body).unwrap();
    let catalog = get_fuse_presets().unwrap();
    assert_eq!(&served, catalog.as_ref());
    assert!(!served.models.is_empty());
    assert!(!served.harnesses.is_empty());
    assert_eq!(script.calls(), 0);
}
