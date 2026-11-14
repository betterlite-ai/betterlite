#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::future::{self, Future};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{StreamExt as _, stream};
use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::{Builder, TempDir};
use tokio::io::{Empty, Sink};

use betterlite_harness::context::{SessionContext, SessionSettings, TurnOutcome};
use betterlite_harness::deepagents::sandbox_backend::TOOL_FILTER_NAME;
use betterlite_harness::deepagents::sandbox_backend::usage::{
    Generation, LlmResult, TokenPricer, UsageCallback,
};
use betterlite_harness::deepagents::transformation::config::DeepAgentsHarnessConfig;
use betterlite_harness::deepagents::transformation::message::{
    Content, Message, MessageKind, MessageUsage, ToolCall, content_text,
};
use betterlite_harness::deepagents::transformation::update::{
    ActionRequest, HitlDecision, INTERRUPT_KEY, Interrupt, InterruptValue, Update, UpdateEntry,
};
use betterlite_harness::deepagents::transformation::{
    BASE_RECURSION_LIMIT, DEEPAGENTS_STEPS_PER_TURN, DEFAULT_RECURSION_LIMIT, INSTALL_HINT,
};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::events::{Approval, Decision, Event};
use betterlite_harness::handlers::BaseHarnessHandler as _;
use betterlite_harness::options::{ClaudeCodeOptions, DeepAgentsOptions, HarnessOptions};
use betterlite_harness::sandbox::{CompletedRun, Process, Sandbox, SandboxCommand};
use betterlite_harness::types::{Harness, JsonObject, PermissionMode, Usage};

use crate::sandbox_backend::Permissions;

use super::DeepAgentsHandler;
use super::graph::{
    AgentRuntime, AgentSpec, Graph, GraphInput, GraphState, GraphStream, MissingRuntime, RunConfig,
    StreamPart,
};

include!("tests/support.rs");
include!("tests/build.rs");
include!("tests/history.rs");
include!("tests/turn.rs");
