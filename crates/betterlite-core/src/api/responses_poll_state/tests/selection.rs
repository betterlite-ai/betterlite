use betterlite_config::api::responses_poll_state::PollingSelector;

use super::super::selection::should_use_polling_for_request;
use super::support::{Watch, deployment_list, projection, providers, selection_enabled, settings};

pub(super) fn requires_conditions() {
    let all = settings(PollingSelector::All, &[]);
    assert!(!selection_enabled(false, "gpt-4o", &all, None));
    assert!(!selection_enabled(
        true,
        "gpt-4o",
        &settings(PollingSelector::Disabled, &[]),
        None
    ));
    assert!(!should_use_polling_for_request(
        true, "gpt-4o", &all, false, None
    ));
    assert!(!selection_enabled(
        true,
        "gpt-4o",
        &settings(PollingSelector::Unrecognized, &[]),
        None
    ));
    assert!(!selection_enabled(
        true,
        "gpt-4o",
        &settings(PollingSelector::Providers(Vec::new()), &["gpt-4o"]),
        None
    ));
    assert!(selection_enabled(true, "gpt-4o", &all, None));
}

pub(super) fn native_mode() {
    let native = settings(PollingSelector::All, &["gpt-4o"]);
    assert!(!selection_enabled(true, "gpt-4o", &native, None));
    assert!(selection_enabled(true, "gpt-5", &native, None));
    assert!(selection_enabled(true, "openai/gpt-4o", &native, None));
}

pub(super) fn prefixed_models() {
    let openai = providers(&["openai"]);
    assert!(selection_enabled(true, "openai/gpt-4o", &openai, None));
    let deployments = deployment_list(&[(Some("openai"), Some("openai/gpt-4o"))]);
    let indices = [0_usize];
    let router = projection(&indices, &deployments);
    assert!(!selection_enabled(
        true,
        "anthropic/model",
        &openai,
        Some(&router)
    ));
    assert!(!selection_enabled(
        true,
        "gpt-4o",
        &providers(&["anthropic"]),
        Some(&router)
    ));
}

pub(super) fn alias_scan() {
    let openai = providers(&["openai"]);
    let deployments = deployment_list(&[
        (Some("anthropic"), Some("openai/gpt-4o")),
        (None, Some("openai/gpt-4o")),
    ]);
    let indices = [0_usize, 1];
    let router = projection(&indices, &deployments);
    assert!(selection_enabled(true, "alias", &openai, Some(&router)));
    assert!(!selection_enabled(true, "alias", &openai, None));
    let no_indices: [usize; 0] = [];
    let empty = projection(&no_indices, &[]);
    assert!(!selection_enabled(true, "alias", &openai, Some(&empty)));
    let custom = deployment_list(&[(Some("anthropic"), Some("openai/gpt-4o"))]);
    let zero = [0_usize];
    let custom_router = projection(&zero, &custom);
    assert!(!selection_enabled(
        true,
        "alias",
        &openai,
        Some(&custom_router)
    ));
    let no_provider = deployment_list(&[(None, Some("gpt-4o"))]);
    let plain_router = projection(&zero, &no_provider);
    assert!(!selection_enabled(
        true,
        "alias",
        &openai,
        Some(&plain_router)
    ));
}

pub(super) fn bad_index() {
    let openai = providers(&["openai"]);
    let deployments = deployment_list(&[(Some("openai"), None)]);
    let indices = [3_usize];
    let missing = projection(&indices, &deployments);
    assert!(!selection_enabled(true, "alias", &openai, Some(&missing)));
    let indices = [0_usize, 3];
    let reaching = projection(&indices, &deployments);
    assert!(selection_enabled(true, "alias", &openai, Some(&reaching)));
}

// deviation: a null deployment model aborts the source's scan; betterlite treats it as absent and continues
pub(super) fn null_deployment_model() {
    let openai = providers(&["openai"]);
    let deployments = deployment_list(&[(None, None), (Some("openai"), None)]);
    let indices = [0_usize, 1];
    let router = projection(&indices, &deployments);
    assert!(selection_enabled(true, "alias", &openai, Some(&router)));
}

pub(super) fn assert_logs(watch: &Watch) {
    assert_eq!(
        watch.messages_at("DEBUG"),
        vec![
            "Model gpt-4o is in native_background_mode list, skipping polling via cache",
            "Polling enabled for model=alias, provider=openai",
            "Could not resolve provider for model alias: list index out of range",
            "Polling enabled for model=alias, provider=openai",
            "Polling enabled for model=alias, provider=openai",
        ]
    );
}
