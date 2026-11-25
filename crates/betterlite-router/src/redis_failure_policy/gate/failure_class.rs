use std::error::Error as StdError;
use std::io;
use std::sync::Arc;

use crate::redis_failure_policy::classify::{FailureClass, MAX_CAUSE_DEPTH};
use redis::{ErrorKind, RedisError, ServerError, ServerErrorKind};
use tokio::time::error::Elapsed;

pub(crate) fn classify(
    error: &(dyn StdError + Send + Sync),
) -> Option<(FailureClass, Vec<FailureClass>)> {
    // the trait's error is not known to be static, so the walk starts at its source; every link after that is static
    let mut current = error.source();
    let mut classes = Vec::new();
    for _ in 0..MAX_CAUSE_DEPTH {
        let Some(link) = current else {
            break;
        };
        if let Some(class) = class_of(link) {
            classes.push(class);
        }
        current = next_cause(link);
    }
    let mut remaining = classes.into_iter();
    let class = remaining.next()?;
    Some((class, remaining.collect()))
}

// The Arc redis holds an io error in and an io::Error both hide the error they wrap from source(), so the walk visits that error directly.
fn next_cause<'error>(
    error: &'error (dyn StdError + 'static),
) -> Option<&'error (dyn StdError + 'static)> {
    // the Arc's Error impl forwards source() to the io error it holds, so the io error is the next link
    if let Some(wrapped) = error.downcast_ref::<Arc<dyn StdError + Send + Sync>>() {
        let link: &'error (dyn StdError + 'static) = wrapped.as_ref();
        return Some(link);
    }
    // std's io error exposes the error it wraps only through get_ref; its source() returns that error's own source
    if let Some(io) = error.downcast_ref::<io::Error>() {
        let link: &'error (dyn StdError + 'static) = io.get_ref()?;
        return Some(link);
    }
    error.source()
}

fn class_of(error: &(dyn StdError + 'static)) -> Option<FailureClass> {
    if let Some(redis) = error.downcast_ref::<RedisError>() {
        let kind = redis.kind();
        if matches!(kind, ErrorKind::Io) {
            return Some(FailureClass::Connection);
        }
        if let ErrorKind::Server(server) = kind {
            return Some(reply_class(Some(server), redis.code(), redis.detail()));
        }
        if matches!(kind, ErrorKind::Extension) {
            return Some(reply_class(None, redis.code(), redis.detail()));
        }
        return Some(FailureClass::Other);
    }
    if let Some(server) = error.downcast_ref::<ServerError>() {
        return Some(reply_class(
            server.kind(),
            Some(server.code()),
            server.details(),
        ));
    }
    if let Some(io) = error.downcast_ref::<io::Error>() {
        return Some(io_class(io));
    }
    if error.downcast_ref::<Elapsed>().is_some() {
        return Some(FailureClass::AsynchronousTimeout);
    }
    None
}

// redis-py maps these replies to ConnectionError subclasses, so LiteLLM counts them as health failures.
fn is_connection_reply(code: Option<&str>, detail: Option<&str>) -> bool {
    match code {
        Some("NOAUTH" | "WRONGPASS") => true,
        Some("ERR") => detail.is_some_and(|text| {
            matches!(
                text,
                "max number of clients reached"
                    | "invalid password"
                    | "AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?"
                    | "Client sent AUTH, but no password is set"
                    | "problem with LDAP service"
            )
        }),
        Some(_) | None => false,
    }
}

// Known reply kinds and extension codes classify the same on both error surfaces.
fn reply_class(
    kind: Option<ServerErrorKind>,
    code: Option<&str>,
    detail: Option<&str>,
) -> FailureClass {
    if kind == Some(ServerErrorKind::BusyLoading) {
        return FailureClass::BusyLoading;
    }
    if matches!(
        kind,
        Some(ServerErrorKind::ClusterDown | ServerErrorKind::MasterDown)
    ) {
        return FailureClass::ClusterDown;
    }
    if is_connection_reply(code, detail) {
        return FailureClass::Connection;
    }
    FailureClass::Command
}

fn io_class(error: &io::Error) -> FailureClass {
    let kind = error.kind();
    if matches!(kind, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) {
        FailureClass::Timeout
    } else if matches!(
        kind,
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
            | io::ErrorKind::NotFound
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::NetworkDown
    ) {
        FailureClass::Connection
    } else {
        FailureClass::Os
    }
}
