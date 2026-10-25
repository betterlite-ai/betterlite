#![cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::future::{self, Future};

use futures_core::Stream;
use futures_util::stream;
use rust_decimal::Decimal;

use crate::context::{SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::handlers::BaseHarnessHandler;
use crate::options::{ClaudeCodeOptions, CodexOptions, HarnessOptions, OpenCodeOptions};
use crate::transformation::{BaseHarnessConfig, HarnessSessionSetup};
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode, Usage};

type Started = Result<(), HarnessError>;
type Turned = Result<Event, HarnessError>;

struct OpenCodeConfig;

impl BaseHarnessConfig for OpenCodeConfig {
    type Options = OpenCodeOptions;
    const HARNESS: Harness = Harness::OpenCode;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([Capability::Resume]),
            permission_modes: BTreeSet::new(),
        }
    }
}

struct OpenCodeHandler {
    config: OpenCodeConfig,
}

impl BaseHarnessHandler for OpenCodeHandler {
    type Config = OpenCodeConfig;

    fn config(&self) -> &OpenCodeConfig {
        &self.config
    }

    fn start<S>(&mut self, _context: &mut SessionContext<S>) -> impl Future<Output = Started> {
        future::ready(Ok(()))
    }

    fn turn<S>(
        &mut self,
        _context: &mut SessionContext<S>,
        _prompt: &str,
    ) -> impl Stream<Item = Turned> {
        stream::empty()
    }

    fn stop<S>(&mut self, _context: &mut SessionContext<S>) -> impl Future<Output = Started> {
        future::ready(Ok(()))
    }

    fn native_session_id(&self) -> Option<String> {
        None
    }

    fn resume<S>(
        &mut self,
        _context: &mut SessionContext<S>,
        _id: &str,
    ) -> impl Future<Output = Started> {
        future::ready(Ok(()))
    }
}

fn context(options: Option<HarnessOptions>) -> SessionContext<()> {
    let mut context = SessionContext {
        harness: Harness::OpenCode,
        sandbox: (),
        session_id: "session".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    context.settings.options = options;
    context
}

#[test]
fn a_new_session_has_litellm_defaults() {
    let SessionContext {
        settings, outcome, ..
    } = context(None);
    assert_eq!(settings.permissions, PermissionMode::Full);
    assert!(settings.model.is_none() && settings.skills.is_empty() && settings.metadata.is_empty());
    let empty = TurnOutcome {
        final_text: String::new(),
        output_json: None,
        usage: Usage::default(),
        cost: Decimal::ZERO,
    };
    assert_eq!(outcome, empty);
}

#[test]
fn get_options_defaults_when_the_session_has_none() {
    let options = OpenCodeConfig.get_options(&context(None)).unwrap();
    assert_eq!(
        options,
        OpenCodeOptions {
            agent: "build".to_owned(),
            config: JsonObject::new(),
            env: BTreeMap::new()
        }
    );
}

#[test]
fn get_options_returns_options_of_its_own_harness() {
    let own = OpenCodeOptions {
        agent: "plan".to_owned(),
        config: JsonObject::new(),
        env: BTreeMap::new(),
    };
    let options = OpenCodeConfig
        .get_options(&context(Some(HarnessOptions::OpenCode(own.clone()))))
        .unwrap();
    assert_eq!(options, own);
}

#[test]
fn validate_environment_rejects_options_of_another_harness() {
    let codex = context(Some(HarnessOptions::Codex(CodexOptions::default())));
    let error = OpenCodeConfig.validate_environment(&codex).unwrap_err();
    assert_eq!(
        error.to_string(),
        "CodexOptions cannot be used with Harness.OPENCODE; use OpenCodeOptions"
    );
}

#[test]
fn option_records_keep_litellm_defaults() {
    assert_eq!(
        ClaudeCodeOptions::default(),
        ClaudeCodeOptions {
            config: JsonObject::new(),
            env: BTreeMap::new()
        }
    );
    let codex = CodexOptions::default();
    assert_eq!(
        (
            codex.reasoning_effort,
            codex.web_search,
            codex.config.len(),
            codex.env.len()
        ),
        (None, false, 0, 0)
    );
    let setup = HarnessSessionSetup::default();
    assert!(
        setup.files.is_empty() && setup.persisted_dirs.is_empty() && setup.skills_dir.is_none()
    );
}

#[tokio::test]
async fn history_is_unsupported_unless_a_handler_overrides_it() {
    let handler = OpenCodeHandler {
        config: OpenCodeConfig,
    };
    assert_eq!(
        handler.config().capabilities().supported,
        BTreeSet::from([Capability::Resume])
    );
    let error = handler.history(&context(None)).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Harness.OPENCODE does not support History"
    );
}
