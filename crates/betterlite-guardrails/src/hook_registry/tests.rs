#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks::{
    self, DuringCall, DuringMcpCall, LoggingOnly, PostCall, PostMcpCall, PreCall, PreMcpCall,
    RealtimeInputTranscription,
};
use betterlite_config::tls::VerifySetting;
use betterlite_http::proxy::base_builder;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use super::error::HookRegistryError;
use super::hooks::supported_event_hooks;
use super::kind::HookKind;
use super::kinds::{
    build_hook_spec, initialize_alice, initialize_cato_networks, initialize_dynamoai,
    initialize_enkryptai,
};
use super::ports::HookPorts;
use super::spec::HookSpec;
use super::wire;
use crate::guardrail_registry::build::GuardrailFactories;
use crate::guardrail_registry::callback::CallbackSpec;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_startup::from_config::skips_startup;
use crate::hook_initializers::azure::AzureArgs;
use crate::hook_initializers::secret_lookup::SecretLookup;
use crate::mcp_jwt_signer_init::JwtSignerSlot;
use crate::tool_policy::{RequestToolNames, ToolPolicyLookup};

fn params_from(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn mode_of(kind: &str) -> &'static str {
    if kind == wire::MCP_JWT_SIGNER {
        "pre_mcp_call"
    } else {
        "pre_call"
    }
}

// every extra the third-party initializers require, plus the fields the ported args copy
fn test_params(kind: &str) -> LitellmParams {
    let mode = mode_of(kind);
    let body = format!(
        r#"{{"guardrail":"{kind}","mode":"{mode}","api_key":"test-key","api_base":"https://guard.example/v1","guard_name":"my-guard","tenant_id":"tenant","client_id":"client","client_secret":"secret","auth_token":"token","base_url":"https://ibm.example/v1","detector_id":"detector","project_id":"project","default_on":true,"timeout":7.5}}"#
    );
    params_from(&body)
}

struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn get_secret_str(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

fn test_ports() -> HookPorts {
    HookPorts {
        secrets: Arc::new(NoSecrets),
        environment: Arc::new(BTreeMap::<String, String>::new()),
        router_present: true,
        slot: Arc::new(JwtSignerSlot::new()),
        registered: Arc::new(Mutex::new(Vec::new())),
        client: Client::default(),
    }
}

fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(SecretString::expose_secret)
}

fn build_ok(kind: &str) -> HookSpec {
    let params = test_params(kind);
    build_hook_spec(kind, &params, "test-guard", &test_ports())
        .expect("wire name is built")
        .expect("initializer succeeds")
}

fn build_error_on(kind: &str, params: &LitellmParams) -> HookRegistryError {
    build_hook_spec(kind, params, "test-guard", &test_ports())
        .expect("wire name is built")
        .err()
        .expect("build fails")
}

fn build_error(kind: &str) -> HookRegistryError {
    let params = test_params(kind);
    build_error_on(kind, &params)
}

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn factories() -> GuardrailFactories {
    GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames))
}

// The supported event hooks of every wire name, written out as each class declares them
const EXPECTED_HOOKS: &[(&str, &[GuardrailEventHooks])] = &[
    (wire::AGENT_365, &[PreMcpCall]),
    (wire::AIM, &[PreCall, DuringCall, PostCall]),
    (wire::AKTO, &[PreCall, PostCall]),
    (wire::ALICE, &[PreCall, DuringCall, PostCall]),
    (wire::APORIA, &[DuringCall, PostCall]),
    (wire::AZURE_PROMPT_SHIELD, &[PreCall, DuringCall]),
    (wire::AZURE_TEXT_MODERATIONS, &[PreCall, DuringCall]),
    (wire::BLOCK_CODE_EXECUTION, &[PreCall, PostCall, DuringCall]),
    (wire::CATO_NETWORKS, &[PreCall, DuringCall, PostCall]),
    (
        wire::CISCO_AI_DEFENSE,
        &[
            PreCall,
            DuringCall,
            PostCall,
            LoggingOnly,
            PreMcpCall,
            DuringMcpCall,
        ],
    ),
    (wire::COMPRESR, &[]),
    (wire::CONDUCT, &[]),
    (wire::CROWDSTRIKE_AIDR, &[PreCall, PostCall]),
    (
        wire::CUSTOM_CODE,
        &[
            PreCall,
            DuringCall,
            PostCall,
            PreMcpCall,
            DuringMcpCall,
            LoggingOnly,
        ],
    ),
    (wire::DEEP_KEEP, &[PreCall, PostCall, DuringCall]),
    (wire::DYNAMOAI, &[PreCall, PostCall, DuringCall]),
    (wire::ENKRYPTAI, &[PreCall, PostCall, DuringCall]),
    (
        wire::GENERIC_GUARDRAIL_API,
        &[PreCall, PostCall, DuringCall],
    ),
    (wire::GRAYSWAN, &[PreCall, DuringCall, PostCall]),
    (wire::GUARDRAILS_AI, &[PostCall, PreCall, LoggingOnly]),
    (wire::HEADROOM, &[PreCall, PostCall]),
    (wire::HIDDENLAYER, &[PreCall, PostCall]),
    (wire::IBM_GUARDRAILS, &[PreCall, PostCall, DuringCall]),
    (wire::JAVELIN, &[PreCall]),
    (wire::LASSO, &[PreCall, DuringCall, PostCall]),
    (
        wire::LITELLM_CONTENT_FILTER,
        &[
            PreCall,
            PostCall,
            DuringCall,
            RealtimeInputTranscription,
            PreMcpCall,
            PostMcpCall,
        ],
    ),
    (wire::LLM_AS_A_JUDGE, &[PreCall, DuringCall, PostCall]),
    (wire::MCP_JWT_SIGNER, &[PreMcpCall]),
    (wire::MICROSOFT_PURVIEW, &[PreCall, PostCall, LoggingOnly]),
    (
        wire::MODEL_ARMOR,
        &[
            PreCall,
            DuringCall,
            PostCall,
            PreMcpCall,
            DuringMcpCall,
            LoggingOnly,
        ],
    ),
    (wire::NOMA, &[PreCall, DuringCall, PostCall, PreMcpCall]),
    (
        wire::NOMA_V2,
        &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall],
    ),
    (wire::ONYX, &[PreCall, DuringCall, PostCall]),
    (wire::OPENAI_MODERATION, &[PreCall, DuringCall, PostCall]),
    (wire::OVALIX, &[PreCall, PostCall]),
    (wire::PANGEA, &[PreCall, PostCall]),
    (
        wire::PANW_PRISMA_AIRS,
        &[
            PreCall,
            DuringCall,
            PostCall,
            LoggingOnly,
            PreMcpCall,
            DuringMcpCall,
            PostMcpCall,
        ],
    ),
    (
        wire::PILLAR,
        &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall],
    ),
    (wire::PROMPTGUARD, &[PreCall, PostCall]),
    (wire::QOSTODIAN_NEXUS, &[]),
    (wire::QUALIFIRE, &[PreCall, DuringCall, PostCall]),
    (wire::REPELLOAI, &[PreCall, PostCall]),
    (wire::RUBRIK, &[]),
    (wire::SEMANTIC_GUARD, &[PreCall, PostCall]),
    (
        wire::SINGULR,
        &[PreCall, PostCall, LoggingOnly, PreMcpCall, PostMcpCall],
    ),
    (wire::STRAIKER, &[PreCall, PostCall]),
    (wire::TYPESAFE, &[]),
    (wire::VIGIL_GUARD, &[PreCall, PostCall]),
    (
        wire::XECGUARD,
        &[PreCall, DuringCall, PostCall, LoggingOnly],
    ),
    (wire::ZSCALER_AI_GUARD, &[PreCall, PostCall]),
];

#[test]
fn every_wire_name_builds_with_its_supported_hooks() {
    assert_eq!(EXPECTED_HOOKS.len(), 50);
    for &(kind, hooks) in EXPECTED_HOOKS {
        let spec = build_ok(kind);
        let settings = spec.settings();
        assert_eq!(
            settings.guardrail_name.as_deref(),
            Some("test-guard"),
            "{kind}"
        );
        assert_eq!(
            settings.event_hook,
            Some(LitellmParamsMode::One(mode_of(kind).to_owned())),
            "{kind}"
        );
        assert!(settings.default_on, "{kind}");
        assert_eq!(settings.timeout, Some(7.5_f64), "{kind}");
        assert_eq!(supported_event_hooks(kind), hooks, "{kind}");
    }
}

#[test]
fn third_party_a_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::AIM, |kind| matches!(kind, HookKind::Aim(_))),
        (wire::AKTO, |kind| matches!(kind, HookKind::Akto(_))),
        (wire::APORIA, |kind| matches!(kind, HookKind::Aporia(_))),
        (wire::CISCO_AI_DEFENSE, |kind| {
            matches!(kind, HookKind::CiscoAiDefense(_))
        }),
        (wire::CONDUCT, |kind| matches!(kind, HookKind::Conduct(_))),
        (wire::CROWDSTRIKE_AIDR, |kind| {
            matches!(kind, HookKind::CrowdstrikeAidr(_))
        }),
        (wire::DEEP_KEEP, |kind| {
            matches!(kind, HookKind::Deepkeep(_))
        }),
        (wire::GENERIC_GUARDRAIL_API, |kind| {
            matches!(kind, HookKind::GenericGuardrailApi(_))
        }),
        (wire::GRAYSWAN, |kind| matches!(kind, HookKind::Grayswan(_))),
        (wire::GUARDRAILS_AI, |kind| {
            matches!(kind, HookKind::GuardrailsAi(_))
        }),
        (wire::HEADROOM, |kind| matches!(kind, HookKind::Headroom(_))),
        (wire::LASSO, |kind| matches!(kind, HookKind::Lasso(_))),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn third_party_b_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::PANW_PRISMA_AIRS, |kind| {
            matches!(kind, HookKind::PanwPrismaAirs(_))
        }),
        (wire::PILLAR, |kind| matches!(kind, HookKind::Pillar(_))),
        (wire::RUBRIK, |kind| matches!(kind, HookKind::Rubrik(_))),
        (wire::STRAIKER, |kind| matches!(kind, HookKind::Straiker(_))),
        (wire::TYPESAFE, |kind| matches!(kind, HookKind::Typesafe(_))),
        (wire::VIGIL_GUARD, |kind| {
            matches!(kind, HookKind::VigilGuard(_))
        }),
        (wire::XECGUARD, |kind| matches!(kind, HookKind::Xecguard(_))),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn hook_initializer_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::AGENT_365, |kind| {
            matches!(kind, HookKind::Agent365(_))
        }),
        (wire::AZURE_PROMPT_SHIELD, |kind| {
            matches!(kind, HookKind::AzurePromptShield(_))
        }),
        (wire::AZURE_TEXT_MODERATIONS, |kind| {
            matches!(kind, HookKind::AzureTextModerations(_))
        }),
        (wire::IBM_GUARDRAILS, |kind| {
            matches!(kind, HookKind::IbmGuardrails(_))
        }),
        (wire::MICROSOFT_PURVIEW, |kind| {
            matches!(kind, HookKind::MicrosoftPurview(_))
        }),
        (wire::MODEL_ARMOR, |kind| {
            matches!(kind, HookKind::ModelArmor(_))
        }),
        (wire::OPENAI_MODERATION, |kind| {
            matches!(kind, HookKind::OpenaiModeration(_))
        }),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn content_filter_and_signer_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::LITELLM_CONTENT_FILTER, |kind| {
            matches!(kind, HookKind::LitellmContentFilter(_))
        }),
        (wire::MCP_JWT_SIGNER, |kind| {
            matches!(kind, HookKind::McpJwtSigner(_))
        }),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn vendor_initializer_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::PROMPTGUARD, |kind| {
            matches!(kind, HookKind::Promptguard(_))
        }),
        (wire::QOSTODIAN_NEXUS, |kind| {
            matches!(kind, HookKind::QostodianNexus(_))
        }),
        (wire::QUALIFIRE, |kind| {
            matches!(kind, HookKind::Qualifire(_))
        }),
        (wire::SEMANTIC_GUARD, |kind| {
            matches!(kind, HookKind::SemanticGuard(_))
        }),
        (wire::SINGULR, |kind| matches!(kind, HookKind::Singulr(_))),
        (wire::ZSCALER_AI_GUARD, |kind| {
            matches!(kind, HookKind::ZscalerAiGuard(_))
        }),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn class_arms_keep_their_params() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::BLOCK_CODE_EXECUTION, |kind| {
            matches!(kind, HookKind::BlockCodeExecution(_))
        }),
        (wire::COMPRESR, |kind| matches!(kind, HookKind::Compresr(_))),
        (wire::CUSTOM_CODE, |kind| {
            matches!(kind, HookKind::CustomCode(_))
        }),
        (wire::HIDDENLAYER, |kind| {
            matches!(kind, HookKind::Hiddenlayer(_))
        }),
        (wire::JAVELIN, |kind| matches!(kind, HookKind::Javelin(_))),
        (wire::LLM_AS_A_JUDGE, |kind| {
            matches!(kind, HookKind::LlmAsAJudge(_))
        }),
        (wire::NOMA, |kind| matches!(kind, HookKind::Noma(_))),
        (wire::NOMA_V2, |kind| matches!(kind, HookKind::NomaV2(_))),
        (wire::ONYX, |kind| matches!(kind, HookKind::Onyx(_))),
        (wire::OVALIX, |kind| matches!(kind, HookKind::Ovalix(_))),
        (wire::PANGEA, |kind| matches!(kind, HookKind::Pangea(_))),
        (wire::REPELLOAI, |kind| {
            matches!(kind, HookKind::Repelloai(_))
        }),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn ported_arms_hold_their_own_args() {
    let cases: &[(&str, fn(&HookKind) -> bool)] = &[
        (wire::ALICE, |kind| matches!(kind, HookKind::Alice(_))),
        (wire::CATO_NETWORKS, |kind| {
            matches!(kind, HookKind::CatoNetworks(_))
        }),
        (wire::DYNAMOAI, |kind| matches!(kind, HookKind::Dynamoai(_))),
        (wire::ENKRYPTAI, |kind| {
            matches!(kind, HookKind::Enkryptai(_))
        }),
    ];
    for &(kind, holds) in cases {
        let spec = build_ok(kind);
        assert!(holds(&spec.kind), "{kind}");
    }
}

#[test]
fn unknown_wire_name_is_not_built() {
    let params = test_params(wire::AIM);
    for kind in ["my_module.MyGuardrail", "bedrock", "tool_policy", ""] {
        assert!(
            build_hook_spec(kind, &params, "test-guard", &test_ports()).is_none(),
            "{kind}"
        );
    }
}

#[test]
fn factory_without_hooks_still_answers_unsupported() {
    let params = test_params(wire::AIM);
    let error = factories()
        .build(wire::AIM, &params, "test-guard", true)
        .err()
        .expect("no builder");
    assert_eq!(error.to_string(), "Unsupported guardrail: aim");
    let RegistryError::Unsupported { kind } = error else {
        panic!("unsupported guardrail expected");
    };
    assert_eq!(kind, wire::AIM);
}

#[test]
fn factory_with_hooks_rejects_unknown_names() {
    let params = params_from(r#"{"guardrail":"my_module.MyGuardrail","mode":"pre_call"}"#);
    let error = factories()
        .with_hooks(test_ports())
        .build("my_module.MyGuardrail", &params, "custom", true)
        .err()
        .expect("unknown kind");
    assert_eq!(
        error.to_string(),
        "Unsupported guardrail: my_module.MyGuardrail"
    );
}

#[test]
fn factory_with_hooks_builds_a_hook_callback() {
    let params = test_params(wire::AIM);
    let mut callbacks = factories()
        .with_hooks(test_ports())
        .build(wire::AIM, &params, "aim-guard", true)
        .unwrap();
    let callback = callbacks.remove(0);
    let CallbackSpec::Hook(spec) = &callback.spec else {
        panic!("hook callback expected");
    };
    assert!(matches!(&spec.kind, HookKind::Aim(_)));
    let settings = &callback.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("aim-guard"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![PreCall, DuringCall, PostCall])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(settings.default_on);
    assert_eq!(settings.timeout, Some(7.5_f64));
    assert!(callback.supports_scan_only_tool_results());
    assert!(!callback.is_compression());
}

#[test]
fn prisma_airs_refuses_scan_only_tool_results() {
    let builder = factories().with_hooks(test_ports());
    let panw = test_params(wire::PANW_PRISMA_AIRS);
    let panw_callbacks = builder
        .build(wire::PANW_PRISMA_AIRS, &panw, "panw-guard", true)
        .unwrap();
    assert!(!panw_callbacks[0].supports_scan_only_tool_results());
    let aim = test_params(wire::AIM);
    let aim_callbacks = builder.build(wire::AIM, &aim, "aim-guard", true).unwrap();
    assert!(aim_callbacks[0].supports_scan_only_tool_results());
}

#[test]
fn initializer_refusal_keeps_its_text() {
    let params = params_from(r#"{"guardrail":"guardrails_ai","mode":"pre_call"}"#);
    let error = build_error_on(wire::GUARDRAILS_AI, &params);
    assert_eq!(
        error.to_string(),
        "GuardrailsAIException - Please pass the Guardrails AI guard name via 'litellm_params::guard_name'"
    );
    let HookRegistryError::Init { kind, .. } = error else {
        panic!("initializer error expected");
    };
    assert_eq!(kind, wire::GUARDRAILS_AI);
}

#[test]
fn unbuilt_decision_model() {
    let error = build_error(wire::DECISION_MODEL);
    assert_eq!(
        error.to_string(),
        "support for the decision_model guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::DECISION_MODEL);
}

#[test]
fn unbuilt_prompt_security() {
    let error = build_error(wire::PROMPT_SECURITY);
    assert_eq!(
        error.to_string(),
        "support for the prompt_security guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::PROMPT_SECURITY);
}

#[test]
fn unbuilt_mcp_end_user_permission() {
    let error = build_error(wire::MCP_END_USER_PERMISSION);
    assert_eq!(
        error.to_string(),
        "support for the mcp_end_user_permission guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::MCP_END_USER_PERMISSION);
}

#[test]
fn unbuilt_mcp_security() {
    let error = build_error(wire::MCP_SECURITY);
    assert_eq!(
        error.to_string(),
        "support for the mcp_security guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::MCP_SECURITY);
}

#[test]
fn hook_errors_at_startup() {
    let init = RegistryError::Hook(HookRegistryError::Init {
        kind: wire::AIM.to_owned(),
        message: "boom".to_owned(),
    });
    let unbuilt = RegistryError::Hook(HookRegistryError::NotBuilt {
        kind: wire::DECISION_MODEL.to_owned(),
    });
    assert!(skips_startup(&init));
    assert!(!skips_startup(&unbuilt));
}

#[test]
fn alice_args_copy_their_fields() {
    let params = test_params(wire::ALICE);
    let args = initialize_alice(&params, Some("alice-guard"));
    assert_eq!(args.guardrail_name, "alice-guard");
    assert_eq!(args.api_base.as_deref(), Some("https://guard.example/v1"));
    assert_eq!(exposed(args.api_key.as_ref()), Some("test-key"));
    assert_eq!(args.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(args.default_on);
    assert_eq!(args.timeout, Some(7.5_f64));
}

#[test]
fn alice_keeps_an_explicit_fail_open() {
    let params = params_from(
        r#"{"guardrail":"alice","mode":"pre_call","unreachable_fallback":"fail_open"}"#,
    );
    let args = initialize_alice(&params, Some("alice-guard"));
    assert_eq!(args.unreachable_fallback, UnreachableFallback::FailOpen);
    assert!(args.api_key.is_none());
}

#[test]
fn cato_networks_reads_ssl_verify() {
    let params =
        params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":false}"#);
    let args = initialize_cato_networks(&params, Some("cato-guard")).expect("args");
    assert_eq!(args.ssl_verify, Some(VerifySetting::Bool(false)));
    assert_eq!(args.inspect_embeddings, None);
    assert_eq!(args.guardrail_name, "cato-guard");
}

#[test]
fn cato_networks_accepts_a_ca_path() {
    let params =
        params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":"/ca.pem"}"#);
    let args = initialize_cato_networks(&params, Some("cato-guard")).expect("args");
    assert_eq!(
        args.ssl_verify,
        Some(VerifySetting::Path("/ca.pem".to_owned()))
    );
}

#[test]
fn cato_networks_rejects_a_ssl_verify_number() {
    let params = params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":7}"#);
    let error = initialize_cato_networks(&params, Some("cato-guard"))
        .err()
        .expect("invalid ssl_verify");
    assert_eq!(
        error.to_string(),
        "the guardrail parameter ssl_verify has an invalid value"
    );
}

#[test]
fn dynamoai_args_copy_their_fields() {
    let params = test_params(wire::DYNAMOAI);
    let args = initialize_dynamoai(&params, Some("dynamo-guard"));
    assert_eq!(args.guardrail_name, "dynamo-guard");
    assert_eq!(args.api_base.as_deref(), Some("https://guard.example/v1"));
    assert_eq!(exposed(args.api_key.as_ref()), Some("test-key"));
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(args.default_on);
    assert_eq!(args.timeout, Some(7.5_f64));
}

#[test]
fn enkryptai_defaults_the_undeclared_params() {
    let params = params_from(r#"{"guardrail":"enkryptai","mode":"pre_call"}"#);
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert_eq!(args.policy_name, None);
    assert_eq!(args.deployment_name, None);
    assert!(args.detectors.is_none());
    assert!(args.block_on_violation);
}

#[test]
fn enkryptai_copies_the_declared_params() {
    let params = params_from(
        r#"{"guardrail":"enkryptai","mode":"pre_call","policy_name":"policy","deployment_name":"deployment","detectors":{"toxicity":true},"block_on_violation":false}"#,
    );
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert_eq!(args.policy_name.as_deref(), Some("policy"));
    assert_eq!(args.deployment_name.as_deref(), Some("deployment"));
    assert_eq!(args.detectors.as_ref().map(BTreeMap::len), Some(1));
    assert!(!args.block_on_violation);
}

#[test]
fn enkryptai_reads_a_null_block_on_violation_as_the_default() {
    let params =
        params_from(r#"{"guardrail":"enkryptai","mode":"pre_call","block_on_violation":null}"#);
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert!(args.block_on_violation);
}

#[test]
fn azure_names_share_one_initializer() {
    let spec = build_ok(wire::AZURE_PROMPT_SHIELD);
    let HookKind::AzurePromptShield(shield) = &spec.kind else {
        panic!("prompt shield spec");
    };
    let AzureArgs::PromptShield(common) = shield.as_ref() else {
        panic!("prompt shield args");
    };
    assert_eq!(common.guardrail_name, "test-guard");

    let moderations_spec = build_ok(wire::AZURE_TEXT_MODERATIONS);
    let HookKind::AzureTextModerations(moderations) = &moderations_spec.kind else {
        panic!("text moderations spec");
    };
    let AzureArgs::TextModerations(text_common) = moderations.as_ref() else {
        panic!("text moderations args");
    };
    assert_eq!(text_common.guardrail_name, "test-guard");
}

#[test]
fn agent_365_args_keep_the_tenant_identity() {
    let spec = build_ok(wire::AGENT_365);
    let HookKind::Agent365(args) = &spec.kind else {
        panic!("agent 365 spec");
    };
    assert_eq!(args.guardrail_name, "test-guard");
    assert_eq!(args.tenant_id, "tenant");
    assert_eq!(args.client_id, "client");
    assert_eq!(exposed(Some(&args.client_secret)), Some("secret"));
}

#[test]
fn agent_365_reports_the_overrides_it_ignores() {
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    let spec = build_ok(wire::AGENT_365);
    drop(guard);
    assert!(matches!(&spec.kind, HookKind::Agent365(_)));
    let printed = String::from_utf8(sink.0.lock().expect("the captured log").clone())
        .expect("the captured log is UTF-8");
    assert!(printed.contains("ignoring api_base"), "{printed}");
}

#[test]
fn mcp_jwt_signer_registers_the_signer_it_builds() {
    let ports = test_ports();
    let params = test_params(wire::MCP_JWT_SIGNER);
    let spec = build_hook_spec(wire::MCP_JWT_SIGNER, &params, "signer-guard", &ports)
        .expect("wire name is built")
        .expect("signer builds");
    let HookKind::McpJwtSigner(signer) = &spec.kind else {
        panic!("signer spec expected");
    };
    let slot = ports.slot.get().expect("slot holds the signer");
    assert!(Arc::ptr_eq(&slot, signer));
    assert_eq!(ports.registered.lock().unwrap().len(), 1);
}

#[test]
fn mcp_jwt_signer_keeps_the_hook_port_client() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-betterlite-port",
        HeaderValue::from_static("hook-registry"),
    );
    let client = base_builder()
        .default_headers(headers)
        .build()
        .expect("a port client");
    let mut ports = test_ports();
    ports.client = client;
    let params = test_params(wire::MCP_JWT_SIGNER);
    let spec = build_hook_spec(wire::MCP_JWT_SIGNER, &params, "signer-guard", &ports)
        .expect("wire name is built")
        .expect("signer builds");
    let HookKind::McpJwtSigner(signer) = &spec.kind else {
        panic!("signer spec expected");
    };
    assert!(
        format!("{:?}", signer.idp.client).contains("x-betterlite-port"),
        "the signer keeps the client the hook port serves"
    );
}

#[test]
fn mcp_jwt_signer_refuses_another_mode() {
    let params = params_from(r#"{"guardrail":"mcp_jwt_signer","mode":"pre_call"}"#);
    let error = build_error_on(wire::MCP_JWT_SIGNER, &params);
    assert!(
        error.to_string().contains("must use mode='pre_mcp_call'"),
        "{error}"
    );
    let HookRegistryError::Init { kind, .. } = error else {
        panic!("initializer error expected");
    };
    assert_eq!(kind, wire::MCP_JWT_SIGNER);
}

#[test]
fn content_filter_requires_a_guardrail_name() {
    let params = test_params(wire::LITELLM_CONTENT_FILTER);
    let error = build_hook_spec(wire::LITELLM_CONTENT_FILTER, &params, "", &test_ports())
        .expect("wire name is built")
        .err()
        .expect("empty name fails");
    assert_eq!(
        error.to_string(),
        "Content Filter: guardrail_name is required"
    );
    assert!(matches!(error, HookRegistryError::Init { .. }));
}

#[test]
fn content_filter_passes_the_router_port() {
    for router_present in [true, false] {
        let mut ports = test_ports();
        ports.router_present = router_present;
        let params = test_params(wire::LITELLM_CONTENT_FILTER);
        let spec = build_hook_spec(wire::LITELLM_CONTENT_FILTER, &params, "test-guard", &ports)
            .expect("wire name is built")
            .expect("content filter builds");
        let HookKind::LitellmContentFilter(args) = &spec.kind else {
            panic!("content filter spec expected");
        };
        assert_eq!(args.router_present, router_present);
    }
}

#[test]
fn semantic_guard_needs_the_router() {
    let mut ports = test_ports();
    ports.router_present = false;
    let params = test_params(wire::SEMANTIC_GUARD);
    let error = build_hook_spec(wire::SEMANTIC_GUARD, &params, "test-guard", &ports)
        .expect("wire name is built")
        .err()
        .expect("missing router fails");
    assert_eq!(
        error.to_string(),
        "SemanticGuard requires llm_router for embeddings. Configure a model_list with an embedding model."
    );
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
