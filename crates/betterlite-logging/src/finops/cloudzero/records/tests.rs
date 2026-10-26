#![cfg(test)]

use chrono::{Datelike as _, Timelike as _};
use rust_decimal::Decimal;

use rstest::rstest;

use super::super::{CbfValue, CzbEntityType, CzrnComponents, CzrnGenerator, UsageRecord};
use super::CbfTransformer;

fn row() -> UsageRecord {
    UsageRecord {
        date: Some("2025-04-19".to_owned()),
        custom_llm_provider: "anthropic".to_owned(),
        team_id: "Team/One".to_owned(),
        team_alias: Some("platform".to_owned()),
        model: "claude-3".to_owned(),
        model_group: "claude".to_owned(),
        api_key: "123456789-secret".to_owned(),
        api_key_alias: Some("prod".to_owned()),
        successful_requests: 1,
        prompt_tokens: 2,
        completion_tokens: 3,
        user_email: None,
        organization_alias: None,
        project_alias: None,
        user_alias: None,
        api_requests: 0,
        failed_requests: 0,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        spend: Decimal::new(125, 2),
    }
}

#[test]
fn cloudzero_czrn_validates_and_extracts_components() {
    let generator = CzrnGenerator;
    assert_eq!(CzbEntityType::Team.as_str(), "team");
    let value = generator
        .create_from_litellm_data(&row())
        .expect("valid CZRN");
    assert_eq!(
        value,
        "czrn:litellm:anthropic:cross-region:team-one:llm-usage:claude-3"
    );
    assert!(generator.is_valid(&value));
    assert!(!generator.is_valid("czrn:bad:format"));
    assert_eq!(
        generator.extract_components(&value).expect("components")[1],
        "anthropic"
    );
}

#[rstest]
#[case::azure("azure", "azure")]
#[case::anthropic("anthropic", "anthropic")]
#[case::vertex_ai("vertex_ai", "vertex-ai")]
#[case::gemini("gemini", "google")]
#[case::cohere("cohere", "cohere")]
#[case::huggingface("huggingface", "huggingface")]
#[case::replicate("replicate", "replicate")]
#[case::together_ai("together_ai", "together-ai")]
fn cloudzero_provider_map_covers_supported_values(#[case] provider: &str, #[case] expected: &str) {
    let generator = CzrnGenerator;
    let mut source = row();
    source.custom_llm_provider = provider.to_owned();
    assert!(
        generator
            .create_from_litellm_data(&source)
            .expect("CZRN")
            .contains(&format!(":{expected}:"))
    );
}

#[test]
fn cloudzero_provider_and_component_normalization_preserve_lookup_order() {
    let generator = CzrnGenerator;
    let mut azure_ai = row();
    azure_ai.custom_llm_provider = "azure_ai".to_owned();
    assert!(
        generator
            .create_from_litellm_data(&azure_ai)
            .expect("CZRN")
            .contains(":azure-ai:")
    );
    let mut bedrock = row();
    bedrock.custom_llm_provider = "bedrock".to_owned();
    assert!(
        generator
            .create_from_litellm_data(&bedrock)
            .expect("CZRN")
            .contains(":aws:")
    );
    let generated = generator
        .create_from_components(CzrnComponents {
            service_type: " LLM ",
            provider: "provider_name",
            region: "US/East",
            owner_account_id: "",
            resource_type: "usage",
            cloud_local_id: "model/x",
        })
        .expect_err("uppercase service type is rejected by the CZRN regex");
    assert_eq!(
        generated,
        super::CzrnError::Invalid(
            "czrn:LLM:provider-name:us-east:unknown:usage:model/x".to_owned()
        )
    );
}

#[test]
fn cloudzero_zero_request_cbf_record_account_formatting_token_tags() {
    let mut zero = row();
    zero.successful_requests = 0;
    let transformer = CbfTransformer {
        czrn_generator: CzrnGenerator,
    };
    let output = transformer.transform(&[zero, row()]);
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("resource/account"),
        Some(&CbfValue::Text("prod|12345678".to_owned()))
    );
    assert_eq!(output[0].get("usage/amount"), Some(&CbfValue::Integer(5)));
    assert_eq!(
        output[0].get("resource/tag:prompt_tokens"),
        Some(&CbfValue::Text("2".to_owned()))
    );
    assert_eq!(
        output[0].get("resource/tag:completion_tokens"),
        Some(&CbfValue::Text("3".to_owned()))
    );
    match output[0].get("time/usage_start") {
        Some(CbfValue::DateTime(date)) => {
            assert_eq!(
                (date.year(), date.month(), date.day(), date.hour()),
                (2_025_i32, 4_u32, 19_u32, 0_u32)
            );
        }
        value => panic!("unexpected date: {value:?}"),
    }
}

#[test]
fn cloudzero_date_and_czrn_failure_filtering() {
    let mut unknown_date = row();
    unknown_date.date = Some("not-a-date".to_owned());
    let mut invalid = row();
    invalid.model = String::new();
    let transformer = CbfTransformer {
        czrn_generator: CzrnGenerator,
    };
    let output = transformer.transform(&[unknown_date, invalid]);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].get("time/usage_start"), Some(&CbfValue::None));
}

#[test]
fn cloudzero_date_parser_accepts_iso_fallback() {
    let mut source = row();
    source.date = Some("2025-04-19T12:30:00+02:00".to_owned());

    let record = CbfTransformer::default()
        .create_cbf_record(&source)
        .expect("valid CBF record");
    assert_eq!(
        record.get("time/usage_start"),
        Some(&CbfValue::DateTime(
            chrono::DateTime::parse_from_rfc3339("2025-04-19T12:30:00+02:00")
                .expect("ISO date")
                .with_timezone(&chrono::Utc),
        ))
    );
}

#[test]
fn cloudzero_cbf_preserves_source_dimensions_as_tags() {
    let mut source = row();
    source.user_email = Some("owner@example.com".to_owned());
    source.organization_alias = Some("acme".to_owned());
    source.project_alias = Some("inference".to_owned());
    source.user_alias = Some("owner".to_owned());
    source.api_requests = 7;
    source.failed_requests = 2;
    source.cache_creation_input_tokens = 3;
    source.cache_read_input_tokens = 4;

    let record = CbfTransformer::default()
        .create_cbf_record(&source)
        .expect("valid CBF record");

    macro_rules! assert_tag {
        ($key:literal, $value:literal) => {
            assert_eq!(
                record.get(concat!("resource/tag:", $key)),
                Some(&CbfValue::Text($value.to_owned()))
            );
        };
    }
    assert_tag!("entity_type", "team");
    assert_tag!("entity_id", "platform");
    assert_tag!("team_alias", "platform");
    assert_tag!("api_key_prefix", "12345678");
    assert_tag!("api_key_alias", "prod");
    assert_tag!("user_email", "owner@example.com");
    assert_tag!("api_requests", "7");
    assert_tag!("successful_requests", "1");
    assert_tag!("failed_requests", "2");
    assert_tag!("cache_creation_tokens", "3");
    assert_tag!("cache_read_tokens", "4");
    assert_tag!("organization_alias", "acme");
    assert_tag!("project_alias", "inference");
    assert_tag!("user_alias", "owner");
}
