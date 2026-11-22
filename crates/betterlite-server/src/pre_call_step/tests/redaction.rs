use std::fs::{File, read_to_string};
use std::sync::Arc;

use tracing::subscriber::set_default;

use super::support::Fake;
use super::{TestKey, behavior, input_for, parts, step_for};
use crate::pre_call::route_type::RouteType;

#[tokio::test]
async fn step_redacts_the_logged_body() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "api_key": "sk-abcdefghijklmnopqrstuvwxyz"}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("log");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(File::create(&path).unwrap()))
        .finish();
    let guard = set_default(subscriber);
    step.run(input.data.clone(), &input, None).await.unwrap();
    drop(guard);
    let written = read_to_string(&path).unwrap();
    assert!(written.contains("receiving data"));
    assert!(written.contains("REDACTED"));
    assert!(!written.contains("sk-abcdefghijklmnopqrstuvwxyz"));
}
