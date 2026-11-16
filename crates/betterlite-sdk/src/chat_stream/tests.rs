#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
use betterlite_error::error::details::{
    MidStreamFallbackDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_logging::fields::usage::Usage as LogUsage;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_streams::assembly::StreamBuild;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::provider_chunk_decode::chunk::{ProviderChunk, ProviderKind};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use indexmap::IndexMap;
use serde_json::from_str;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::ChatStream;
use super::config::{CallMode, ChatStreamConfig, REPEATED_CHUNK_LIMIT, StreamLoggingFlags};
use super::error::ChatStreamError;
use super::ports::{
    ChatStreamPorts, ChunkSource, SourceFailure, StreamCache, StreamClock, StreamFailure,
    StreamLogging, StreamSourceError, StreamingDeploymentHook, SystemClock,
};
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CorrelationRestore;
use crate::call_setup::hooks::HookError;

include!("tests/support.rs");
include!("tests/next.rs");
include!("tests/duration.rs");
include!("tests/finalize.rs");
include!("tests/failure.rs");
include!("tests/fallback.rs");
include!("tests/mcp.rs");
include!("tests/close.rs");
