#![cfg(test)]

use super::{
    ChatAnnotation, ImageItem, ImageUrl, UrlCitation, citation_fields as project_citations,
    image_finish_status as project_status, image_results as project_images,
    usage_details as project_usage,
};
use betterlite_models::decisions::Json as ModelJson;
use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
};
use betterlite_openai::api::responses_shapes::OutputCallStatus;
use rstest::rstest;
use std::collections::BTreeMap;

#[test]
fn image_results() {
    let images = [
        ImageItem {
            image_url: Some(ImageUrl {
                url: Some("data:image/png;base64,YQ==".to_owned()),
            }),
        },
        ImageItem {
            image_url: Some(ImageUrl {
                url: Some("https://a.test/image".to_owned()),
            }),
        },
    ];
    let items = project_images(Some(&images), Some("stop"), || "fixed".to_owned());
    assert_eq!(items.len(), 2);
    let first = items.first().expect("first image item");
    let second = items.get(1).expect("second image item");
    assert_eq!(first.result.as_deref(), Some("YQ=="));
    assert_eq!(second.result.as_deref(), Some("https://a.test/image"));
    assert_eq!(first.id, "ig_fixed");
    assert_eq!(first.status, OutputCallStatus::Completed);
    assert!(project_images(None, None, || "unused".to_owned()).is_empty());
}

#[test]
fn image_results_preserves_order_and_first_comma() {
    let multi = [
        ImageItem {
            image_url: Some(ImageUrl {
                url: Some("YQ==".to_owned()),
            }),
        },
        ImageItem {
            image_url: Some(ImageUrl {
                url: Some("data:x,a,b".to_owned()),
            }),
        },
    ];
    let output = project_images(Some(&multi), None, || "next".to_owned());
    assert_eq!(
        output.first().expect("first image item").result.as_deref(),
        Some("YQ==")
    );
    assert_eq!(
        output.get(1).expect("second image item").result.as_deref(),
        Some("a,b")
    );
}

#[rstest]
#[case::empty("")]
#[case::data_without_comma("data:missing-comma")]
#[case::empty_data_suffix("data:image/png;base64,")]
fn image_results_drop_empty_results(#[case] url: &str) {
    let images = [ImageItem {
        image_url: Some(ImageUrl {
            url: Some(url.to_owned()),
        }),
    }];
    assert!(project_images(Some(&images), None, || "unused".to_owned()).is_empty());
}

#[test]
fn image_finish_status() {
    assert_eq!(project_status(Some("length")), OutputCallStatus::Incomplete);
    assert_eq!(project_status(Some("error")), OutputCallStatus::Failed);
    assert_eq!(
        project_status(Some("tool_calls")),
        OutputCallStatus::Completed
    );
    assert_eq!(project_status(None), OutputCallStatus::Completed);
    assert_eq!(
        project_status(Some("content_filter")),
        OutputCallStatus::Failed
    );
}

#[test]
fn citation_fields() {
    let annotations = [ChatAnnotation {
        r#type: Some("url_citation".to_owned()),
        url_citation: Some(UrlCitation {
            start_index: Some(1),
            end_index: Some(4),
            url: Some("https://a.test".to_owned()),
            title: Some("A".to_owned()),
        }),
    }];
    let projected = project_citations(Some(&annotations));
    assert_eq!(projected.len(), 1);
    let citation = projected.first().expect("citation item");
    assert_eq!(citation.start_index, Some(1));
    assert_eq!(citation.title.as_deref(), Some("A"));
    let empty = [ChatAnnotation {
        r#type: Some("url_citation".to_owned()),
        url_citation: Some(UrlCitation {
            start_index: None,
            end_index: None,
            url: None,
            title: None,
        }),
    }];
    let empty_projection = project_citations(Some(&empty));
    assert_eq!(
        empty_projection.first().expect("empty citation item").url,
        None
    );
    assert!(project_citations(None).is_empty());
    let missing_nested = [ChatAnnotation {
        r#type: Some("url_citation".to_owned()),
        url_citation: None,
    }];
    assert!(project_citations(Some(&missing_nested)).is_empty());
}

#[test]
fn usage_details() {
    let usage = Usage {
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 99,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    let projected = project_usage(Some(&usage));
    assert_eq!(projected.input_tokens, 2);
    assert_eq!(projected.total_tokens, 99);
    let empty = project_usage(None);
    assert_eq!(empty.total_tokens, 0);
}

#[test]
fn usage_details_projects_typed_counters() {
    let usage = detailed_usage();
    let projected = project_usage(Some(&usage));
    assert_eq!(projected.cost, Some(rust_decimal::Decimal::new(125, 3)));
    let input = projected.input_tokens_details.expect("input details");
    assert_eq!(input.cached_tokens, 0);
    assert_eq!(
        input.extra.get("cache_write_tokens"),
        Some(&ModelJson::Number(serde_json::Number::from(7_i64)))
    );
    assert_eq!(
        input.extra.get("web_search_requests"),
        Some(&ModelJson::Number(serde_json::Number::from(0_i64)))
    );
    assert_eq!(
        input
            .cached_tokens_details
            .expect("typed cached counters")
            .text_tokens,
        Some(6)
    );
    let output = projected.output_tokens_details.expect("output details");
    assert_eq!(output.reasoning_tokens, Some(0));
    assert_eq!(output.audio_tokens, Some(0));
    assert_eq!(
        output.extra.get("image_tokens"),
        Some(&ModelJson::Number(serde_json::Number::from(2_i64)))
    );
}

fn detailed_usage() -> Usage {
    let prompt_tokens_details = PromptTokensDetails {
        text_tokens: Some(5),
        audio_tokens: Some(0),
        image_tokens: Some(2),
        video_tokens: Some(1),
        web_search_requests: Some(0),
        google_maps_grounding_requests: Some(2),
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: Some(0),
        cache_creation_tokens: Some(7),
        cached_tokens: None,
        cached_tokens_details: Some(cached_tokens_details()),
    };
    Usage {
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 99,
        prompt_tokens_details: Some(prompt_tokens_details),
        completion_tokens_details: Some(CompletionTokensDetails {
            text_tokens: Some(5),
            audio_tokens: Some(0),
            image_tokens: Some(2),
            video_tokens: None,
            reasoning_tokens: None,
            accepted_prediction_tokens: None,
            rejected_prediction_tokens: None,
        }),
        numeric_extras: BTreeMap::from([(
            "cost".to_owned(),
            UsageNumber::Decimal(rust_decimal::Decimal::new(125, 3)),
        )]),
    }
}

fn cached_tokens_details() -> CachedTokensDetails {
    CachedTokensDetails {
        text_tokens: Some(6),
        audio_tokens: Some(1),
        image_tokens: Some(2),
    }
}
