#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper as _;
use serde_json::{Value, json};

use super::{Input, Op};

fn listing(input: Value) -> Value {
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    Op::run(&parsed).unwrap()
}

fn defaults() -> Value {
    json!({
        "prometheus_metrics_config": [],
        "prometheus_exclude_metrics": [],
        "prometheus_exclude_labels": [],
        "prometheus_latency_buckets": null,
        "prometheus_emit_input_sequence_length_label": false,
        "caller_identity": "api_key_alias",
    })
}

fn collectors(output: &Value) -> Vec<Value> {
    output["collectors"].as_array().unwrap().clone()
}

fn named(collectors: &[Value], name: &str) -> Value {
    collectors
        .iter()
        .find(|collector| collector["name"] == name)
        .unwrap()
        .clone()
}

#[test]
fn the_default_listing_holds_the_81_collectors_in_litellm_order() {
    let collectors = collectors(&listing(defaults()));
    assert_eq!(collectors.len(), 81);
    assert_eq!(
        named(&collectors, "litellm_proxy_failed_requests_metric")["kind"],
        "counter"
    );
    assert_eq!(collectors[80]["name"], "litellm_mcp_tool_call_spend_metric");
}

#[test]
fn a_custom_latency_bucket_list_reaches_only_the_latency_histograms() {
    let mut input = defaults();
    input["prometheus_latency_buckets"] = json!([0.5, 2.5]);
    let collectors = collectors(&listing(input));
    assert_eq!(
        named(&collectors, "litellm_request_total_latency_metric")["buckets"],
        json!([0.5, 2.5, "+Inf"])
    );
    assert_eq!(
        named(&collectors, "litellm_deployment_latency_per_output_token")["buckets"][0],
        json!(0.005)
    );
}

#[test]
fn an_excluded_metric_is_absent_from_the_listing() {
    let mut input = defaults();
    input["prometheus_exclude_metrics"] = json!(["litellm_spend_metric"]);
    let collectors = collectors(&listing(input));
    assert_eq!(collectors.len(), 80);
    assert!(
        collectors
            .iter()
            .all(|collector| collector["name"] != "litellm_spend_metric")
    );
}
