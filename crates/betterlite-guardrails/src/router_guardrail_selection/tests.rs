#![cfg(test)]

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::iter::repeat_with;

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rand::SeedableRng as _;
use rand::TryRng;
use rand::rngs::StdRng;

use super::error::GuardrailSelectionError;
use super::select_router_guardrail;
use crate::guardrail_startup::router_list::RouterGuardrail;

fn guardrail(name: &str, id: &str) -> RouterGuardrail {
    RouterGuardrail {
        guardrail_name: name.to_owned(),
        id: Some(id.to_owned()),
        guardrail: "presidio".to_owned(),
        mode: LitellmParamsMode::One("pre_call".to_owned()),
        api_key: None,
        api_base: None,
    }
}

struct CountingRng {
    draws: u64,
}

impl TryRng for CountingRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.draws = self.draws.saturating_add(1);
        Ok(0)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        self.draws = self.draws.saturating_add(1);
        Ok(0)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        self.draws = self.draws.saturating_add(1);
        dst.fill(0);
        Ok(())
    }
}

#[test]
fn select_returns_the_only_entry_with_the_name_without_drawing() {
    let guardrails = [
        guardrail("pii", "one"),
        guardrail("toxic", "two"),
        guardrail("pii-extra", "three"),
    ];
    let mut rng = CountingRng { draws: 0 };

    let picked = select_router_guardrail(&guardrails, "toxic", &mut rng).unwrap();

    assert_eq!(picked.id.as_deref(), Some("two"));
    assert_eq!(rng.draws, 0);
}

#[test]
fn select_fails_with_litellms_message_when_no_name_matches() {
    let guardrails = [guardrail("pii", "one"), guardrail("PII", "two")];
    let mut rng = StdRng::seed_from_u64(1);

    let error = select_router_guardrail(&guardrails, "pi", &mut rng).unwrap_err();

    assert!(matches!(error, GuardrailSelectionError::NotFound { .. }));
    assert_eq!(error.to_string(), "No guardrail found with name: pi");
}

#[test]
fn select_fails_on_an_empty_list() {
    let mut rng = StdRng::seed_from_u64(1);

    let error = select_router_guardrail(&[], "pii", &mut rng).unwrap_err();

    assert_eq!(error.to_string(), "No guardrail found with name: pii");
}

#[test]
fn select_can_pick_every_entry_of_a_shared_name_and_no_other() {
    let guardrails = [
        guardrail("pii", "first"),
        guardrail("toxic", "other"),
        guardrail("pii", "second"),
        guardrail("pii", "third"),
    ];
    let mut rng = StdRng::seed_from_u64(7);

    let picked: BTreeSet<&str> = repeat_with(|| {
        let chosen = select_router_guardrail(&guardrails, "pii", &mut rng).unwrap();
        chosen.id.as_deref().unwrap()
    })
    .take(200)
    .collect();

    assert_eq!(picked, BTreeSet::from(["first", "second", "third"]));
}

#[test]
fn select_draws_from_the_random_source_for_a_shared_name() {
    let guardrails = [guardrail("pii", "first"), guardrail("pii", "second")];
    let mut rng = CountingRng { draws: 0 };

    let picked = select_router_guardrail(&guardrails, "pii", &mut rng).unwrap();

    assert!(rng.draws > 0);
    assert_eq!(picked.guardrail_name, "pii");
}
