use indexmap::IndexMap;
use redis::Value;
use serde::Serialize;

use super::RedisAdminError;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum InfoValue {
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Map(IndexMap<String, Self>),
    Bool(bool),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientListError {
    #[error("a client list token has no key before it")]
    OrphanToken,
}

// redis-py 8.1.0 redis/_parsers/helpers.py:parse_info
#[must_use]
pub fn parse_info(text: &str) -> IndexMap<String, InfoValue> {
    let mut info = IndexMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((first_key, first_value)) = line.split_once(':') else {
            push_info_item(&mut info, "__raw__", InfoValue::Text(line.to_owned()));
            continue;
        };
        let (key, value) = match line.rsplit_once(':') {
            // cmdstat_host is the one key whose value holds a colon as well
            Some((last_key, last_value)) if first_key == "cmdstat_host" => (last_key, last_value),
            _ => (first_key, first_value),
        };
        if key == "module" {
            push_info_item(&mut info, "modules", info_value(value));
        } else {
            info.insert(key.to_owned(), info_value(value));
        }
    }
    info
}

fn push_info_item(info: &mut IndexMap<String, InfoValue>, key: &str, value: InfoValue) {
    match info.get_mut(key) {
        Some(InfoValue::List(items)) => items.push(value),
        _ => {
            info.insert(key.to_owned(), InfoValue::List(vec![value]));
        }
    }
}

fn info_value(value: &str) -> InfoValue {
    if !value.contains(',') && !value.contains('=') {
        if value.contains('.') {
            return value
                .parse::<f64>()
                .map_or_else(|_error| InfoValue::Text(value.to_owned()), InfoValue::Float);
        }
        return value
            .parse::<i64>()
            .map_or_else(|_error| InfoValue::Text(value.to_owned()), InfoValue::Int);
    }
    if !value.contains('=') {
        return InfoValue::List(
            value
                .split(',')
                .filter(|item| !item.is_empty())
                .map(info_value)
                .collect(),
        );
    }
    let mut map = IndexMap::new();
    for item in value.split(',') {
        if item.is_empty() {
            continue;
        }
        match item.rsplit_once('=') {
            Some((key, item_value)) => {
                map.insert(key.to_owned(), info_value(item_value));
            }
            None => {
                map.insert(item.to_owned(), InfoValue::Bool(true));
            }
        }
    }
    InfoValue::Map(map)
}

// redis-py 8.1.0 redis/_parsers/helpers.py:parse_client_list
pub fn parse_client_list(text: &str) -> Result<Vec<IndexMap<String, String>>, ClientListError> {
    let mut clients = Vec::new();
    for line in text.lines() {
        let mut client = IndexMap::new();
        let mut last_key: Option<String> = None;
        for token in line.split(' ') {
            match token.split_once('=') {
                Some((key, value)) => {
                    client.insert(key.to_owned(), value.to_owned());
                    last_key = Some(key.to_owned());
                }
                None => append_token(&mut client, last_key.as_deref(), token)?,
            }
        }
        if !client.is_empty() {
            clients.push(client);
        }
    }
    Ok(clients)
}

// an address on a unix socket can hold a space, so a bare token extends the value before it
fn append_token(
    client: &mut IndexMap<String, String>,
    last_key: Option<&str>,
    token: &str,
) -> Result<(), ClientListError> {
    let Some(key) = last_key else {
        return Err(ClientListError::OrphanToken);
    };
    let Some(existing) = client.get_mut(key) else {
        return Err(ClientListError::OrphanToken);
    };
    existing.push(' ');
    existing.push_str(token);
    Ok(())
}

// redis-rs merges an INFO sent to every cluster primary into one map of node to text
pub(super) fn first_node_text(reply: Value) -> Result<String, RedisAdminError> {
    let Value::Map(entries) = reply else {
        return Err(cluster_info_error());
    };
    for (_node, value) in entries {
        if let Ok(text) = redis::from_redis_value::<String>(value) {
            return Ok(text);
        }
    }
    Err(cluster_info_error())
}

fn cluster_info_error() -> RedisAdminError {
    RedisAdminError::from(ClientReuseError::from(LiveClientError::Redis(
        redis::RedisError::from((
            redis::ErrorKind::UnexpectedReturnType,
            "the redis cluster INFO reply carried no node text",
        )),
    )))
}
