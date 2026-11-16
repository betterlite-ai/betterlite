use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities as _, ReasoningError};

use crate::chat::capabilities::{CostMap, supports_stop_reason};

use super::support::{Fixture, capabilities, catalog, object, runtime};

// The routing entry answers most lookups, while the bare entry holds the OpenAI catalog's answers
const PREFERRED: &str = r#"{
  "azure_ai/gpt-5": {
    "litellm_provider": "azure_ai",
    "mode": "chat",
    "supports_tool_choice": false,
    "supports_none_reasoning_effort": true
  },
  "gpt-5": {
    "litellm_provider": "openai",
    "mode": "chat",
    "supports_tool_choice": true,
    "supports_none_reasoning_effort": false
  }
}"#;

const BARE_ONLY: &str = r#"{
  "gpt-5-mini": {
    "litellm_provider": "openai",
    "mode": "chat",
    "supports_tool_choice": true,
    "supports_none_reasoning_effort": false
  }
}"#;

const DECLARED_MEDIUM: &str = r#"{
  "azure_ai/gpt-5": {
    "litellm_provider": "azure_ai",
    "mode": "chat",
    "supports_tool_choice": true,
    "supports_none_reasoning_effort": true,
    "default_reasoning_effort": "medium"
  },
  "gpt-5": {
    "litellm_provider": "openai",
    "mode": "chat",
    "supports_tool_choice": true,
    "supports_none_reasoning_effort": true
  }
}"#;

const PLAIN: &str = r#"{
  "azure_ai/gpt-4o": {
    "litellm_provider": "azure_ai",
    "mode": "chat",
    "supports_tool_choice": false
  },
  "gpt-4o": {
    "litellm_provider": "openai",
    "mode": "chat",
    "supports_tool_choice": true
  }
}"#;

const GROK: &str = r#"{
  "azure_ai/grok-4": {"supports_tool_choice": true},
  "grok-4": {"supports_tool_choice": true},
  "azure_ai/grok-3": {"supports_tool_choice": true},
  "grok-3": {"supports_tool_choice": true}
}"#;

const RESPONSE_SCHEMA: &str = r#"{
  "gpt-5": {"supports_response_schema": true},
  "gpt-5-mini": {"supports_response_schema": false}
}"#;

const REMOVED: [&str; 9] = [
    "tool_choice",
    "presence_penalty",
    "frequency_penalty",
    "stop",
    "logit_bias",
    "modalities",
    "prediction",
    "audio",
    "web_search_options",
];

const KEPT: [&str; 6] = [
    "reasoning_effort",
    "verbosity",
    "max_completion_tokens",
    "logprobs",
    "top_logprobs",
    "top_p",
];

pub(super) fn gpt5_capability_selection() {
    routed_entry_wins();
    bare_catalog_gate();
    declared_default_effort();
    gpt5_mapping();
    ordinary_model_filtering();
}

fn routed_entry_wins() {
    let mut fixture = capabilities(PREFERRED, &["gpt-5"], &[]);
    let params = fixture.supported("gpt-5", &catalog(&[], &[]));
    for kept in KEPT {
        assert!(params.contains(&kept), "{kept} is supported");
    }
    for removed in REMOVED {
        assert!(!params.contains(&removed), "{removed} is removed");
    }
    assert_eq!(fixture.lookup_name("gpt-5"), "azure_ai/gpt-5");
    assert_eq!(fixture.lookup_name("azure_ai/gpt-5"), "azure_ai/gpt-5");
    assert_eq!(fixture.lookup_name("gpt-5-mini"), "gpt-5-mini");
}

fn bare_catalog_gate() {
    let plain = catalog(&[], &[]);
    let mut listed = capabilities(BARE_ONLY, &["gpt-5-mini"], &[]);
    let listed_params = listed.supported("gpt-5-mini", &plain);
    assert!(listed_params.contains(&"tool_choice"));
    for unsupported in ["logprobs", "top_logprobs", "top_p"] {
        assert!(!listed_params.contains(&unsupported));
    }
    let mut unlisted = capabilities(BARE_ONLY, &[], &[]);
    let unlisted_params = unlisted.supported("gpt-5-mini", &plain);
    assert!(!unlisted_params.contains(&"tool_choice"));
}

fn declared_default_effort() {
    let plain = catalog(&[], &[]);
    let mut declared = capabilities(DECLARED_MEDIUM, &["gpt-5"], &[]);
    let error = declared
        .mapped(
            "gpt-5",
            r#"{"temperature":0.5}"#,
            DropParams::default(),
            &plain,
        )
        .expect_err("a declared effort of medium refuses a non-default temperature");
    assert!(matches!(
        error,
        ReasoningError::UnsupportedTemperature { .. }
    ));
    assert_eq!(
        error.to_string(),
        "gpt-5 does not support temperature=0.5 while reasoning is active"
    );
    let mut preferred = capabilities(PREFERRED, &["gpt-5"], &[]);
    let optional = preferred
        .mapped(
            "gpt-5",
            r#"{"temperature":0.5,"logprobs":true}"#,
            DropParams::default(),
            &plain,
        )
        .expect("an undeclared default resolves to the supported none effort");
    assert_eq!(optional, object(r#"{"temperature":0.5,"logprobs":true}"#));
}

fn gpt5_mapping() {
    let plain = catalog(&[], &[]);
    let mut preferred = capabilities(PREFERRED, &["gpt-5"], &[]);
    let renamed = preferred
        .mapped(
            "gpt-5",
            r#"{"max_tokens":64,"temperature":1,"reasoning_effort":{"effort":"none"}}"#,
            DropParams::default(),
            &plain,
        )
        .expect("the gpt-5 request maps");
    assert_eq!(
        renamed,
        object(r#"{"max_completion_tokens":64,"temperature":1,"reasoning_effort":"none"}"#)
    );
    sampling_guards(&mut preferred, &plain);
}

fn sampling_guards(preferred: &mut Fixture, plain: &CatalogCapabilities) {
    let error = preferred
        .mapped(
            "gpt-5",
            r#"{"reasoning_effort":"low","logprobs":true}"#,
            DropParams::default(),
            plain,
        )
        .expect_err("sampling params need a none effort");
    assert!(matches!(error, ReasoningError::UnsupportedSampling { .. }));
    let dropped = preferred
        .mapped(
            "gpt-5",
            r#"{"reasoning_effort":"low","logprobs":true}"#,
            DropParams {
                global: false,
                request: true,
            },
            plain,
        )
        .expect("drop_params discards the sampling params");
    assert_eq!(dropped, object(r#"{"reasoning_effort":"low"}"#));
    let mut bare = capabilities(BARE_ONLY, &["gpt-5-mini"], &[]);
    let globally_dropped = bare
        .mapped(
            "gpt-5-mini",
            r#"{"temperature":0.5,"logprobs":true}"#,
            DropParams {
                global: true,
                request: false,
            },
            plain,
        )
        .expect("the global drop discards both guards");
    assert_eq!(globally_dropped, Object::new());
}

fn ordinary_model_filtering() {
    let with_user = catalog(&["gpt-4o"], &[]);
    let mut plain_model = capabilities(PLAIN, &["gpt-4o"], &[]);
    let params = plain_model.supported("gpt-4o", &with_user);
    assert!(!params.contains(&"tool_choice"));
    assert!(params.contains(&"stop"));
    assert!(params.contains(&"user"));
    assert!(!params.contains(&"reasoning_effort"));
    // an uncataloged model gains reasoning_effort only under the OpenAI family, never Azure AI Studio
    let uncataloged = plain_model.supported("gpt-4o", &catalog(&[], &[]));
    assert!(!uncataloged.contains(&"reasoning_effort"));
    // the superclass map reads the plain GPT allowlist, so tool_choice survives the filtered query
    let optional = plain_model
        .mapped(
            "gpt-4o",
            r#"{"temperature":0.5,"tool_choice":"auto","stop":["x"],"user":"u","nonsense":1}"#,
            DropParams::default(),
            &with_user,
        )
        .expect("the ordinary request maps");
    assert_eq!(
        optional,
        object(r#"{"temperature":0.5,"tool_choice":"auto","stop":["x"],"user":"u"}"#)
    );
    stop_reason_delegation();
}

fn stop_reason_delegation() {
    assert!(!supports_stop_reason("grok-4"));
    assert!(!supports_stop_reason("grok-3-mini"));
    assert!(!supports_stop_reason("grok-code-fast-1"));
    assert!(supports_stop_reason("grok-3"));
    // the delegation is case-sensitive, so a differently cased name never reaches the xAI rule
    assert!(supports_stop_reason("Grok-4"));
    assert!(supports_stop_reason("gpt-4o"));
    let mut grok = capabilities(GROK, &["grok-4", "grok-3"], &[]);
    let grok_catalog = catalog(&["grok-4", "grok-3"], &[]);
    let grok_four = grok.supported("grok-4", &grok_catalog);
    assert!(!grok_four.contains(&"stop"));
    assert!(grok_four.contains(&"tool_choice"));
    let grok_three = grok.supported("grok-3", &grok_catalog);
    assert!(grok_three.contains(&"stop"));
}

#[test]
fn catalog_cost_map_reads_the_model_catalog() {
    let model_catalog = ModelCatalog::parse(br#"{"gpt-4o":{}}"#, UnknownFields::Reject)
        .expect("the fixture catalog parses");
    assert!(CostMap::contains(&model_catalog, "gpt-4o"));
    assert!(!CostMap::contains(&model_catalog, "gpt-5"));
}

#[test]
fn wrapper_forwards_schema_and_default_declarations() {
    let mut fixture = capabilities(RESPONSE_SCHEMA, &["gpt-5", "gpt-5-mini"], &[]);
    let schema_model = CapabilityRequest {
        model: "gpt-5",
        custom_llm_provider: None,
    };
    let plain_model = CapabilityRequest {
        model: "gpt-5-mini",
        custom_llm_provider: None,
    };
    let mut wrapper = fixture.wrapper();
    assert!(runtime().block_on(wrapper.supports_response_schema(&schema_model)));
    assert!(!runtime().block_on(wrapper.supports_response_schema(&plain_model)));
    assert!(!wrapper.catalogue_declares_default());
    let mut declared = capabilities(DECLARED_MEDIUM, &["gpt-5"], &[]);
    assert!(declared.wrapper().catalogue_declares_default());
}
