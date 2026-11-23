use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::Branch;
use super::support::{Recorder, call, params, run, settings, text};

pub(super) async fn kwargs_guardrails_fallback() {
    let dispatch = Recorder::default();
    let mut connect = params();
    connect.guardrails = Some(Json::List(vec![text("second")]));
    connect.litellm_metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(vec![text("third")]),
    )]));
    let litellm_params = IndexMap::new();
    let request = call("meta/muse-1", &connect, None, &litellm_params, false);
    let (_, result) = run(&dispatch, &settings(), &request).await;
    let connection = result.unwrap();
    let Branch::ProviderConfig { tag, .. } = &connection.branch else {
        panic!("a meta model must take the generic provider-config branch");
    };
    assert_eq!(*tag, RealtimeConfigTag::Meta);
    let metadata = connection.args.litellm_metadata.as_ref().unwrap();
    assert_eq!(
        metadata.get("guardrails"),
        Some(&Json::List(vec![text("second")]))
    );
}
