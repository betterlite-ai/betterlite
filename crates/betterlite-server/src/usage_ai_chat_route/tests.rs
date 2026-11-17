#![cfg(test)]

mod support;

include!("tests/route.rs");
include!("tests/body.rs");
include!("tests/caller.rs");
include!("tests/stream.rs");
include!("tests/headers.rs");
include!("tests/oneshot.rs");
include!("tests/corpus.rs");

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;
use serde_json::json;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::keepalive_interval::PingSetting;
use crate::usage_ai_chat::messages::ChatMessage;
use crate::usage_ai_chat_route::caller::{GuardCallerId, identity};

use support::{
    ANSWER_FRAME, DONE_FRAME, FakeCallerId, FakeFactory, FakeModel, FakeSource, PING_FRAME,
    THINKING_FRAME, admin_principal, answer_body, answer_reply, body_text, frames,
    key_admin_principal, master_principal, next_frame, parsed, post, principal, send, state,
    usage_tool_reply, with_principal,
};
