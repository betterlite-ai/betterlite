#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, IntoFuture as _, ready};
use std::io;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode as AxumStatus, Uri};
use axum::routing::post;
use betterlite_config::sections::grayswan::{Configured, OnFlaggedAction, ReasoningMode};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::{RouteCatalog, RouteEntry};
use futures_util::stream::iter;
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::subscriber::{set_default, with_default};

use super::super::grayswan_monitoring::Object;
use super::super::grayswan_monitoring::call::{MAX_RESPONSE_BYTES, call_grayswan_api};
use super::super::grayswan_monitoring::context::{
    PostCallContextRequest, StructuredMessageSource, post_call_context,
};
use super::super::grayswan_monitoring::failure::{GuardrailFailureLog, log_guardrail_failure};
use super::super::grayswan_monitoring::headers::{
    FORWARDED_HEADER_NAMES, extract_inbound_headers, prepare_headers,
};
use super::super::grayswan_monitoring::helpers::{
    GuardrailException, MessageRow, as_plain_dict, is_grayswan_exception,
};
use super::super::grayswan_monitoring::payload::{PreparePayloadRequest, prepare_payload};
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::grayswan::construction::{resolve_reasoning_mode, resolve_threshold};
use crate::grayswan::error::{
    GRAYSWAN_BLOCK_ERROR_MSG, GraySwanFailure, GraySwanGuardrailAPIError,
};
use crate::grayswan::wire::{GraySwanDynamicParams, GraySwanPayload, GraySwanResponse};
use crate::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use crate::guardrail_translation::{BaseTranslation, TranslationCall};
use crate::unified_dispatch::registry::{TranslationEntry, TranslationRegistry};

use super::{configured, field, message, plain_message};

#[path = "tests/apply.rs"]
mod apply;

include!("tests/support.rs");
include!("tests/helpers.rs");
include!("tests/headers.rs");
include!("tests/payload.rs");
include!("tests/context.rs");
include!("tests/transport.rs");
include!("tests/failure_log.rs");
