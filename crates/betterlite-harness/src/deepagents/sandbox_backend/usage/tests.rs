#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Metadata, Subscriber};

use crate::deepagents::transformation::message::{Content, Message, MessageKind, MessageUsage};
use crate::types::Usage;

use super::{
    AccountingError, Generation, LlmResult, TokenPricer, UsageCallback, UsageLedger, UsageTotals,
    message_cost, record_llm_usage,
};

#[derive(Default)]
struct FixedPrice {
    price: Option<Decimal>,
    lookups: AtomicUsize,
    last: Mutex<Option<(String, u64, u64)>>,
}

impl TokenPricer for FixedPrice {
    fn cost_per_token(
        &self,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Option<Decimal> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        *self.last.lock().unwrap() = Some((model.to_owned(), prompt_tokens, completion_tokens));
        self.price
    }
}

fn price(price: Option<Decimal>) -> Arc<FixedPrice> {
    Arc::new(FixedPrice {
        price,
        lookups: AtomicUsize::new(0),
        last: Mutex::new(None),
    })
}

fn message_with(usage: MessageUsage, reported_cost: Option<Decimal>) -> Message {
    Message {
        kind: MessageKind::Ai,
        content: Content::Empty,
        reasoning_content: String::new(),
        tool_calls: Vec::new(),
        tool_call_id: String::new(),
        name: None,
        is_error: false,
        usage,
        reported_cost,
    }
}

fn generation(usage: MessageUsage, reported_cost: Option<Decimal>) -> Generation {
    Generation {
        message: Some(message_with(usage, reported_cost)),
    }
}

fn one_call(usage: MessageUsage, reported_cost: Option<Decimal>) -> LlmResult {
    LlmResult {
        generations: vec![vec![generation(usage, reported_cost)]],
    }
}

fn empty_ledger() -> UsageLedger {
    UsageLedger {
        usage: Usage::default(),
        cost: Decimal::ZERO,
        unpriced: BTreeSet::new(),
    }
}

#[test]
fn message_cost_prefers_the_reported_response_cost() {
    let pricer = price(Some(Decimal::new(90, 1)));
    let message = message_with(MessageUsage::default(), Some(Decimal::new(15, 1)));
    let cost = message_cost(
        &message,
        Some("openai/gpt-4o"),
        MessageUsage::default(),
        pricer.as_ref(),
    );
    assert_eq!(cost, Some(Decimal::new(15, 1)));
    assert_eq!(pricer.lookups.load(Ordering::Relaxed), 0);
}

#[test]
fn message_cost_prices_the_tokens_when_no_cost_is_reported() {
    let pricer = price(Some(Decimal::new(5, 0)));
    let message = message_with(MessageUsage::default(), None);
    let tokens = MessageUsage {
        input_tokens: 2,
        output_tokens: 3,
    };
    let cost = message_cost(&message, Some("openai/gpt-4o"), tokens, pricer.as_ref());
    assert_eq!(cost, Some(Decimal::new(5, 0)));
    assert_eq!(
        *pricer.last.lock().unwrap(),
        Some(("openai/gpt-4o".to_owned(), 2, 3))
    );
}

#[test]
fn message_cost_is_none_without_a_cost_model() {
    let pricer = price(Some(Decimal::new(5, 0)));
    let message = message_with(MessageUsage::default(), None);
    assert_eq!(
        message_cost(&message, None, MessageUsage::default(), pricer.as_ref()),
        None
    );
    let unpriced = price(None);
    assert_eq!(
        message_cost(
            &message,
            Some("unknown/model"),
            MessageUsage::default(),
            unpriced.as_ref()
        ),
        None
    );
}

#[test]
fn record_llm_usage_counts_one_call_and_sums_tokens() {
    let mut ledger = empty_ledger();
    let result = LlmResult {
        generations: vec![
            vec![generation(
                MessageUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                },
                None,
            )],
            vec![generation(
                MessageUsage {
                    input_tokens: 4,
                    output_tokens: 5,
                },
                None,
            )],
        ],
    };
    record_llm_usage(&mut ledger, None, &result, price(None).as_ref()).unwrap();
    assert_eq!(
        ledger.usage,
        Usage {
            input_tokens: 6,
            output_tokens: 8,
            calls: 1,
        }
    );
    assert_eq!(ledger.cost, Decimal::ZERO);
}

#[test]
fn record_llm_usage_counts_a_call_without_generations() {
    let mut ledger = empty_ledger();
    let result = LlmResult {
        generations: vec![vec![Generation { message: None }]],
    };
    record_llm_usage(&mut ledger, None, &result, price(None).as_ref()).unwrap();
    assert_eq!(ledger.usage.calls, 1);
    assert_eq!(ledger.usage.input_tokens, 0);
}

#[test]
fn record_llm_usage_adds_reported_and_priced_costs() {
    let pricer = price(Some(Decimal::new(3, 0)));
    let mut ledger = empty_ledger();
    let result = LlmResult {
        generations: vec![vec![
            generation(MessageUsage::default(), Some(Decimal::new(25, 1))),
            generation(MessageUsage::default(), None),
        ]],
    };
    record_llm_usage(&mut ledger, Some("openai/gpt-4o"), &result, pricer.as_ref()).unwrap();
    assert_eq!(ledger.cost, Decimal::new(55, 1));
}

#[test]
fn record_llm_usage_looks_up_an_unpriced_model_only_once() {
    let pricer = price(None);
    let mut ledger = empty_ledger();
    let result = one_call(MessageUsage::default(), None);
    record_llm_usage(&mut ledger, Some("unknown/model"), &result, pricer.as_ref()).unwrap();
    record_llm_usage(&mut ledger, Some("unknown/model"), &result, pricer.as_ref()).unwrap();
    assert_eq!(pricer.lookups.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.cost, Decimal::ZERO);
    assert!(ledger.unpriced.contains("unknown/model"));
}

#[test]
fn record_llm_usage_counts_the_call_before_a_token_overflow() {
    let mut ledger = UsageLedger {
        usage: Usage {
            input_tokens: u64::MAX,
            output_tokens: 0,
            calls: 0,
        },
        cost: Decimal::ZERO,
        unpriced: BTreeSet::new(),
    };
    let result = one_call(
        MessageUsage {
            input_tokens: 1,
            output_tokens: 0,
        },
        None,
    );
    assert_eq!(
        record_llm_usage(&mut ledger, None, &result, price(None).as_ref()),
        Err(AccountingError::TokensOverflow)
    );
    assert_eq!(ledger.usage.calls, 1);
}

#[test]
fn record_llm_usage_reports_a_call_count_overflow() {
    let mut ledger = UsageLedger {
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
            calls: u64::MAX,
        },
        cost: Decimal::ZERO,
        unpriced: BTreeSet::new(),
    };
    let result = one_call(MessageUsage::default(), None);
    assert_eq!(
        record_llm_usage(&mut ledger, None, &result, price(None).as_ref()),
        Err(AccountingError::CallsOverflow)
    );
}

#[test]
fn record_llm_usage_reports_a_cost_overflow() {
    let pricer = price(Some(Decimal::MAX));
    let mut ledger = UsageLedger {
        usage: Usage::default(),
        cost: Decimal::MAX,
        unpriced: BTreeSet::new(),
    };
    let result = one_call(MessageUsage::default(), None);
    assert_eq!(
        record_llm_usage(&mut ledger, Some("openai/gpt-4o"), &result, pricer.as_ref()),
        Err(AccountingError::CostOverflow)
    );
}

#[test]
fn usage_callback_accumulates_across_shares() {
    let callback = UsageCallback::new(
        Some("openai/gpt-4o".to_owned()),
        price(Some(Decimal::new(2, 0))),
    );
    let shared = callback.clone();
    let result = one_call(
        MessageUsage {
            input_tokens: 3,
            output_tokens: 4,
        },
        None,
    );
    shared.record(&result);
    callback.record(&result);
    assert_eq!(
        callback.totals(),
        UsageTotals {
            usage: Usage {
                input_tokens: 6,
                output_tokens: 8,
                calls: 2,
            },
            cost: Decimal::new(4, 0),
        }
    );
}

#[test]
fn usage_callback_swallows_accounting_errors() {
    let callback = UsageCallback::new(None, price(None));
    let result = one_call(
        MessageUsage {
            input_tokens: u64::MAX,
            output_tokens: 0,
        },
        None,
    );
    callback.record(&result);
    callback.record(&result);
    assert_eq!(callback.totals().usage.calls, 2);
    assert_eq!(callback.totals().usage.input_tokens, u64::MAX);
}

struct WarningCounter {
    warnings: Arc<AtomicUsize>,
}

impl Subscriber for WarningCounter {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if *event.metadata().level() == Level::WARN {
            self.warnings.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[test]
fn an_unpriced_model_is_logged_once() {
    let warnings = Arc::new(AtomicUsize::new(0));
    let subscriber = WarningCounter {
        warnings: Arc::clone(&warnings),
    };
    with_default(subscriber, || {
        let pricer = price(None);
        let mut ledger = empty_ledger();
        let result = one_call(MessageUsage::default(), None);
        record_llm_usage(&mut ledger, Some("unknown/model"), &result, pricer.as_ref()).unwrap();
        record_llm_usage(&mut ledger, Some("unknown/model"), &result, pricer.as_ref()).unwrap();
    });
    assert_eq!(warnings.load(Ordering::Relaxed), 1);
}
