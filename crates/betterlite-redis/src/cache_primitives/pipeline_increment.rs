mod tests;

use super::namespace::check_and_fix_namespace;

#[derive(Debug, Clone)]
pub struct IncrementOperation {
    pub key: String,
    pub increment_value: f64,
    pub ttl: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineStep {
    IncrByFloat { key: String, value: f64 },
    Expire { key: String, ttl: i64 },
}

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

// the driver already classifies each untyped reply, so a non-float EXPIRE reply is None here
#[must_use]
pub fn float_replies(replies: &[Option<f64>]) -> Vec<f64> {
    let mut floats = Vec::with_capacity(replies.len());
    for reply in replies.iter().flatten() {
        floats.push(*reply);
    }
    floats
}
