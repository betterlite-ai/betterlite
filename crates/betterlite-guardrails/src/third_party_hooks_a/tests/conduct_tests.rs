use axum::http::StatusCode;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use crate::conduct::error::ConductCheckError;
use crate::third_party_hooks_a::conduct;
use crate::third_party_hooks_a::error::ThirdPartyHookError;
use crate::third_party_init_a::conduct::ConductArgs;

#[test]
fn conduct_build_reports_the_missing_package() {
    let args = ConductArgs {
        name: "conduct".to_owned(),
        api_url: None,
        agent_token: None,
        workspace_id: None,
        tool_name: "llm_call".to_owned(),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: 8.0,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    };
    let error = conduct::build(&args).expect_err("conduct never builds");
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(
        &error,
        ThirdPartyHookError::ConductMissingPackage(ConductCheckError::MissingPackage)
    ));
    assert!(
        error
            .to_string()
            .contains("conduct-litellm-guard>=0.2.5 is required")
    );
}
