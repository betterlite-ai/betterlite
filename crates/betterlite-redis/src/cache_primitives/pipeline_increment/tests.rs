#![cfg(test)]

use super::{IncrementOperation, PipelineStep, float_replies, plan_pipeline_steps};

#[test]
fn chains_incrbyfloat_and_expire_in_operation_order() {
    let operations = [
        IncrementOperation {
            key: "a".to_owned(),
            increment_value: 1.0_f64,
            ttl: Some(30),
        },
        IncrementOperation {
            key: "b".to_owned(),
            increment_value: 2.5_f64,
            ttl: None,
        },
    ];
    assert_eq!(
        plan_pipeline_steps(&operations, None),
        [
            PipelineStep::IncrByFloat {
                key: "a".to_owned(),
                value: 1.0_f64
            },
            PipelineStep::Expire {
                key: "a".to_owned(),
                ttl: 30
            },
            PipelineStep::IncrByFloat {
                key: "b".to_owned(),
                value: 2.5_f64
            },
        ]
    );
}

#[test]
fn namespaces_every_operation_key() {
    let operations = [IncrementOperation {
        key: "a".to_owned(),
        increment_value: 1.0_f64,
        ttl: None,
    }];
    assert_eq!(
        plan_pipeline_steps(&operations, Some("cache")),
        [PipelineStep::IncrByFloat {
            key: "cache:a".to_owned(),
            value: 1.0_f64
        }]
    );
}

#[test]
fn returns_an_empty_plan_for_an_empty_batch() {
    assert_eq!(plan_pipeline_steps(&[], Some("cache")), []);
}

#[test]
fn float_replies_keeps_only_the_classified_floats_in_order() {
    let replies = [Some(1.0_f64), None, Some(2.5_f64), None];
    assert_eq!(float_replies(&replies), [1.0_f64, 2.5_f64]);
}

#[test]
fn float_replies_is_empty_when_there_are_no_floats() {
    assert_eq!(float_replies(&[None]), Vec::<f64>::new());
}
