mod tests;

use super::namespace::check_and_fix_namespace;

/// One entry of the batch this helper pipelines.
///
/// Mirrors `LiteLLM`'s `RedisPipelineIncrementOperation` `TypedDict`.
#[derive(Debug, Clone)]
pub struct IncrementOperation {
    pub key: String,
    pub increment_value: f64,
    pub ttl: Option<i64>,
}

/// One command this helper queues onto the pipeline, in the order it must be sent.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineStep {
    IncrByFloat { key: String, value: f64 },
    Expire { key: String, ttl: i64 },
}

/// Namespaces each operation's key and chains its `INCRBYFLOAT`.
///
/// Followed by an `EXPIRE` whenever that operation names a TTL, in operation order.
#[must_use]
pub fn plan_pipeline_steps(
    operations: &[IncrementOperation],
    namespace: Option<&str>,
) -> Vec<PipelineStep> {
    let mut steps = Vec::new();
    for operation in operations {
        let namespaced_key = check_and_fix_namespace(namespace, Some(&operation.key))
            .unwrap_or_else(|| operation.key.clone());
        steps.push(PipelineStep::IncrByFloat {
            key: namespaced_key.clone(),
            value: operation.increment_value,
        });
        if let Some(ttl) = operation.ttl {
            steps.push(PipelineStep::Expire {
                key: namespaced_key,
                ttl,
            });
        }
    }
    steps
}

/// Keeps only the `INCRBYFLOAT` replies.
///
/// The driver classifies each untyped reply as it decodes it, so a non-float reply (the
/// `EXPIRE` booleans) already arrives as `None` here, mirroring
/// `[r for r in results if isinstance(r, float)]`.
#[must_use]
pub fn float_replies(replies: &[Option<f64>]) -> Vec<f64> {
    replies.iter().filter_map(|reply| *reply).collect()
}
