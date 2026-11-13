use crate::arguments::json::Object;
use crate::cache_defaults::attribution::record_gateway_injection;

use super::support::{json, object, wire};

fn record(params: &str, added: usize) -> Object {
    let mut record = object(params);
    record_gateway_injection(&mut record, added);
    record
}

#[test]
fn cache_attribution_names_the_serving_deployment() {
    assert_eq!(
        wire(&record(
            r#"{"litellm_metadata": {}, "model_info": {"id": "deployment-a"}}"#,
            1
        )),
        json(
            r#"{"litellm_metadata": {"litellm_gateway_injected_cache": "deployment-a"}, "model_info": {"id": "deployment-a"}}"#
        )
    );
}

#[test]
fn cache_attribution_prefers_litellm_metadata_over_metadata() {
    assert_eq!(
        wire(&record(
            r#"{"litellm_metadata": {}, "metadata": {}, "model_info": {"id": "deployment-a"}}"#,
            1
        )),
        json(
            r#"{"litellm_metadata": {"litellm_gateway_injected_cache": "deployment-a"}, "metadata": {}, "model_info": {"id": "deployment-a"}}"#
        )
    );
}

#[test]
fn cache_attribution_falls_back_to_the_metadata_bucket() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {}, "model_info": {"id": "deployment-a"}}"#,
            1
        )),
        json(
            r#"{"metadata": {"litellm_gateway_injected_cache": "deployment-a"}, "model_info": {"id": "deployment-a"}}"#
        )
    );
}

#[test]
fn cache_attribution_skips_a_bucket_that_is_not_an_object() {
    assert_eq!(
        wire(&record(
            r#"{"litellm_metadata": "nope", "metadata": {}, "model_info": {"id": "deployment-a"}}"#,
            1
        )),
        json(
            r#"{"litellm_metadata": "nope", "metadata": {"litellm_gateway_injected_cache": "deployment-a"}, "model_info": {"id": "deployment-a"}}"#
        )
    );
}

#[test]
fn cache_attribution_creates_no_bucket() {
    assert_eq!(
        wire(&record(r#"{"model_info": {"id": "deployment-a"}}"#, 1)),
        json(r#"{"model_info": {"id": "deployment-a"}}"#)
    );
}

#[test]
fn cache_attribution_preserves_the_every_deployment_sentinel() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {"litellm_gateway_injected_cache": ""}, "model_info": {"id": "deployment-a"}}"#,
            1
        )),
        json(
            r#"{"metadata": {"litellm_gateway_injected_cache": ""}, "model_info": {"id": "deployment-a"}}"#
        )
    );
}

#[test]
fn a_later_deployment_overwrites_an_earlier_mark() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {"litellm_gateway_injected_cache": "deployment-a"}, "model_info": {"id": "deployment-b"}}"#,
            1
        )),
        json(
            r#"{"metadata": {"litellm_gateway_injected_cache": "deployment-b"}, "model_info": {"id": "deployment-b"}}"#
        )
    );
}

#[test]
fn cache_attribution_is_skipped_for_no_new_breakpoints() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {}, "model_info": {"id": "deployment-a"}}"#,
            0
        )),
        json(r#"{"metadata": {}, "model_info": {"id": "deployment-a"}}"#)
    );
}

#[test]
fn a_non_object_model_info_marks_the_request_for_every_deployment() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {}, "model_info": "deployment-a"}"#,
            1
        )),
        json(
            r#"{"metadata": {"litellm_gateway_injected_cache": ""}, "model_info": "deployment-a"}"#
        )
    );
}

#[test]
fn model_info_without_an_id_marks_the_request_for_every_deployment() {
    assert_eq!(
        wire(&record(r#"{"metadata": {}, "model_info": {}}"#, 1)),
        json(r#"{"metadata": {"litellm_gateway_injected_cache": ""}, "model_info": {}}"#)
    );
}

#[test]
fn a_null_deployment_id_is_kept_as_null() {
    assert_eq!(
        wire(&record(
            r#"{"metadata": {}, "model_info": {"id": null}}"#,
            1
        )),
        json(
            r#"{"metadata": {"litellm_gateway_injected_cache": null}, "model_info": {"id": null}}"#
        )
    );
}

#[test]
fn a_numeric_deployment_id_stays_a_number() {
    assert_eq!(
        wire(&record(r#"{"metadata": {}, "model_info": {"id": 0}}"#, 1)),
        json(r#"{"metadata": {"litellm_gateway_injected_cache": 0}, "model_info": {"id": 0}}"#)
    );
}
