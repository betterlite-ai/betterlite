#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_core::api::request_destination::RequestData;

use crate::limiter_records::{
    RateLimitDescriptor, RateLimitDescriptorRateLimit, TagRateLimit, TagRateLimitResolver,
};

use super::create_tag_descriptors;

struct Fixed {
    limits: BTreeMap<String, TagRateLimit>,
    calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl TagRateLimitResolver for Fixed {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        self.calls
            .lock()
            .expect("the resolver calls lock")
            .push(tag_names.to_vec());
        self.limits.clone()
    }
}

fn resolver(entries: &[(&str, Option<i64>, Option<i64>)]) -> Fixed {
    let limits = entries
        .iter()
        .map(|(tag, rpm_limit, tpm_limit)| {
            (
                (*tag).to_owned(),
                TagRateLimit {
                    rpm_limit: *rpm_limit,
                    tpm_limit: *tpm_limit,
                },
            )
        })
        .collect();
    Fixed {
        limits,
        calls: Mutex::new(Vec::new()),
    }
}

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

fn descriptor(tag: &str, requests: Option<i64>, tokens: Option<i64>) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "tag".to_owned(),
        value: tag.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: requests,
            tokens_per_unit: tokens,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn calls(resolver: &Fixed) -> Vec<Vec<String>> {
    resolver
        .calls
        .lock()
        .expect("the resolver calls lock")
        .clone()
}

#[tokio::test]
async fn creates_a_descriptor_for_each_mapped_tag() {
    let resolver = resolver(&[("tag-a", Some(10), Some(20)), ("tag-b", Some(1), None)]);
    let descriptors =
        create_tag_descriptors(&data(r#"{"tags": ["tag-a", "tag-b"]}"#), 60, &resolver).await;
    assert_eq!(
        descriptors,
        vec![
            descriptor("tag-a", Some(10), Some(20)),
            descriptor("tag-b", Some(1), None),
        ]
    );
}

#[tokio::test]
async fn filters_tags_the_resolver_does_not_map() {
    let resolver = resolver(&[("tag-a", Some(10), None)]);
    let descriptors =
        create_tag_descriptors(&data(r#"{"tags": ["tag-a", "tag-b"]}"#), 60, &resolver).await;
    assert_eq!(descriptors, vec![descriptor("tag-a", Some(10), None)]);
    assert_eq!(
        calls(&resolver),
        vec![vec!["tag-a".to_owned(), "tag-b".to_owned()]]
    );
}

#[tokio::test]
async fn deduplicates_tags_before_resolving() {
    let resolver = resolver(&[("tag-a", Some(10), None)]);
    let descriptors =
        create_tag_descriptors(&data(r#"{"tags": ["tag-a", "tag-a"]}"#), 60, &resolver).await;
    assert_eq!(descriptors, vec![descriptor("tag-a", Some(10), None)]);
    assert_eq!(calls(&resolver), vec![vec!["tag-a".to_owned()]]);
}

#[tokio::test]
async fn does_not_call_the_resolver_without_tags() {
    let resolver = resolver(&[("tag-a", Some(10), None)]);
    let descriptors = create_tag_descriptors(&data("{}"), 60, &resolver).await;
    assert!(descriptors.is_empty());
    assert!(calls(&resolver).is_empty());
}

#[tokio::test]
async fn follows_the_request_tag_order() {
    let resolver = resolver(&[("tag-a", Some(1), None), ("tag-b", Some(2), None)]);
    let descriptors =
        create_tag_descriptors(&data(r#"{"tags": ["tag-b", "tag-a"]}"#), 60, &resolver).await;
    assert_eq!(descriptors[0].value, "tag-b");
    assert_eq!(descriptors[1].value, "tag-a");
}

#[tokio::test]
async fn keeps_zero_limits() {
    let resolver = resolver(&[("tag-a", Some(0), Some(0))]);
    let descriptors = create_tag_descriptors(&data(r#"{"tags": ["tag-a"]}"#), 60, &resolver).await;
    assert_eq!(descriptors, vec![descriptor("tag-a", Some(0), Some(0))]);
}
