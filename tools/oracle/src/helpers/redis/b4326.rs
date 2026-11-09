use std::error::Error;
use std::future::{Future, ready};
use std::io;
use std::sync::{Mutex, MutexGuard};

use betterlite_redis::cache_primitives::protocol::{AsyncRedisCommands, RedisScalar};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_admin::connection_test::connection_test_result;
use betterlite_redis::redis_cache_admin::health::ping_commands;
use betterlite_redis::redis_cache_admin::info_parse::{
    ClientListError, parse_client_list, parse_info,
};
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::RedisError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ParseInfo {
        text: String,
    },
    ParseClientList {
        text: String,
    },
    ConnectionTest {
        family: Family,
        ping: PingScript,
        #[serde(default)]
        error_text: String,
    },
    PingEvents {
        outcome: Outcome,
        #[serde(default)]
        error_text: String,
    },
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    Standalone,
    Cluster,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PingScript {
    Answer(bool),
    Failure(String),
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Success,
    Error,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ParseInfo { text } => Ok(json!(parse_info(text))),
            Input::ParseClientList { text } => Ok(match parse_client_list(text) {
                Ok(clients) => json!(clients),
                Err(ClientListError::OrphanToken) => json!({"error": "orphan_token"}),
            }),
            Input::ConnectionTest {
                family,
                ping,
                error_text,
            } => {
                let answer = match ping {
                    PingScript::Answer(answer) => Ok(*answer),
                    PingScript::Failure(name) if name == "error" => {
                        Err(RedisAdminError::from(LiveClientError::Redis(
                            RedisError::from(io::Error::other(error_text.clone())),
                        )))
                    }
                    PingScript::Failure(name) => {
                        return Err(failure("InvalidInput", format!("unsupported ping {name}")));
                    }
                };
                Ok(json!(connection_test_result(family.family(), answer)))
            }
            Input::PingEvents {
                outcome,
                error_text,
            } => ping_events(*outcome, error_text),
        }
    }
}

impl Family {
    const fn family(self) -> RedisCacheFamily {
        match self {
            Self::Standalone => RedisCacheFamily::Standalone,
            Self::Cluster => RedisCacheFamily::Cluster,
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

struct ScriptedPing {
    answer: Result<bool, RedisError>,
}

impl ScriptedPing {
    fn reject(&self) -> LiveClientError {
        match &self.answer {
            Ok(_answer) => LiveClientError::Redis(RedisError::from(io::Error::other(
                "the scripted client only answers ping".to_owned(),
            ))),
            Err(error) => LiveClientError::Redis(error.clone()),
        }
    }
}

impl AsyncRedisCommands for ScriptedPing {
    type Error = LiveClientError;
    type Reply = ();
    type Pipeline = ();

    fn ping(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        match &self.answer {
            Ok(answer) => ready(Ok(*answer)),
            Err(error) => ready(Err(LiveClientError::Redis(error.clone()))),
        }
    }

    fn delete(&self, _names: &[&str]) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn ttl(&self, _name: &str) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn expire(
        &self,
        _name: &str,
        _time: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn rpush(
        &self,
        _name: &str,
        _values: &[RedisScalar],
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn lpop(
        &self,
        _name: &str,
        _count: Option<i64>,
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn pipeline(&self, _transaction: bool) -> Self::Pipeline {}

    fn eval(
        &self,
        _script: &str,
        _numkeys: i64,
        _keys_and_args: &[RedisScalar],
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Err(self.reject()))
    }
}

#[derive(Default)]
struct RecordingHooks {
    events: Mutex<Vec<Value>>,
}

impl RecordingHooks {
    fn log(&self) -> MutexGuard<'_, Vec<Value>> {
        match self.events.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl ServiceHooks for RecordingHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log().push(
            json!({"call_type": event.call_type, "outcome": "success", "error": Value::Null}),
        );
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log().push(json!({
            "call_type": event.call_type,
            "outcome": "failure",
            "error": error.to_string(),
        }));
    }
}

fn ping_events(outcome: Outcome, error_text: &str) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("Runtime", error.to_string()))?;
    let hooks = RecordingHooks::default();
    let client = ScriptedPing {
        answer: match outcome {
            Outcome::Success => Ok(true),
            Outcome::Error => Err(RedisError::from(io::Error::other(error_text.to_owned()))),
        },
    };
    let answer = runtime.block_on(ping_commands(&client, &hooks, None));
    let (result, error) = match answer {
        Ok(value) => (json!(value), Value::Null),
        Err(error) => (Value::Null, json!(error.to_string())),
    };
    let events = hooks.log().clone();
    Ok(json!({"result": result, "error": error, "events": events}))
}
