#![cfg(test)]

use super::fixtures;
use crate::investigation::loop_state::{self, Context};
use crate::models::settings::Check;

#[tokio::test]
async fn prompt_over_the_character_limit_exits_without_calling_the_model() {
    let mut claim = fixtures::claim();
    claim.job.settings.checks = (0..40_u32)
        .map(|index| Check {
            id: format!("check-{index}"),
            instruction: "x".repeat(3000),
            enabled: true,
        })
        .collect();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert!(prompts.lock().unwrap().is_empty());
}
