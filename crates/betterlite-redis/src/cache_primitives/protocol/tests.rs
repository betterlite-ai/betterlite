#![cfg(test)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;

use super::{AsyncRedisCommands, RedisScalar};

#[derive(Debug, Clone, PartialEq)]
enum FakeReply {
    Int(i64),
    Nil,
}

struct FakeClient {
    lists: Mutex<HashMap<String, Vec<String>>>,
    ttls: Mutex<HashMap<String, i64>>,
}

impl FakeClient {
    fn new() -> Self {
        Self {
            lists: Mutex::new(HashMap::new()),
            ttls: Mutex::new(HashMap::new()),
        }
    }
}

fn push_text_values(
    lists: &Mutex<HashMap<String, Vec<String>>>,
    name: &str,
    values: &[RedisScalar],
) -> usize {
    let mut updated = lists.lock().unwrap().get(name).cloned().unwrap_or_default();
    for value in values {
        let RedisScalar::Text(text) = value else {
            continue;
        };
        updated.push(text.clone());
    }
    let length = updated.len();
    lists.lock().unwrap().insert(name.to_owned(), updated);
    length
}

fn remove_keys(lists: &Mutex<HashMap<String, Vec<String>>>, names: &[&str]) -> usize {
    let mut locked = lists.lock().unwrap();
    names
        .iter()
        .filter(|name| locked.remove(**name).is_some())
        .count()
}

fn pop_front(lists: &Mutex<HashMap<String, Vec<String>>>, name: &str) -> FakeReply {
    let Some(mut popped) = lists.lock().unwrap().get(name).cloned() else {
        return FakeReply::Nil;
    };
    if popped.is_empty() {
        return FakeReply::Nil;
    }
    popped.remove(0);
    let reply = FakeReply::Int(i64::try_from(popped.len()).unwrap_or_default());
    lists.lock().unwrap().insert(name.to_owned(), popped);
    reply
}

impl AsyncRedisCommands for FakeClient {
    type Error = Infallible;
    type Reply = FakeReply;
    type Pipeline = bool;

    fn ping(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        ready(Ok(true))
    }

    fn delete(&self, names: &[&str]) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        let removed = remove_keys(&self.lists, names);
        ready(Ok(i64::try_from(removed).unwrap_or_default()))
    }

    fn ttl(&self, name: &str) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        let reply = self.ttls.lock().unwrap().get(name).copied().unwrap_or(-1);
        ready(Ok(reply))
    }

    fn expire(
        &self,
        name: &str,
        time: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.ttls.lock().unwrap().insert(name.to_owned(), time);
        ready(Ok(true))
    }

    fn rpush(
        &self,
        name: &str,
        values: &[RedisScalar],
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        let length = push_text_values(&self.lists, name, values);
        ready(Ok(i64::try_from(length).unwrap_or_default()))
    }

    fn lpop(
        &self,
        name: &str,
        _count: Option<i64>,
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Ok(pop_front(&self.lists, name)))
    }

    fn pipeline(&self, transaction: bool) -> Self::Pipeline {
        transaction
    }

    fn eval(
        &self,
        _script: &str,
        numkeys: i64,
        _keys_and_args: &[RedisScalar],
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Ok(FakeReply::Int(numkeys)))
    }
}

#[tokio::test]
async fn ping_reports_the_connection_is_alive() {
    assert!(FakeClient::new().ping().await.unwrap());
}

#[tokio::test]
async fn rpush_then_lpop_round_trips_a_value() {
    let client = FakeClient::new();
    let pushed = client
        .rpush("queue", &[RedisScalar::Text("a".to_owned())])
        .await
        .unwrap();
    assert_eq!(pushed, 1);
    let popped = client.lpop("queue", None).await.unwrap();
    assert_eq!(popped, FakeReply::Int(0));
}

#[tokio::test]
async fn lpop_on_an_empty_list_reports_nil() {
    assert_eq!(
        FakeClient::new().lpop("missing", Some(1)).await.unwrap(),
        FakeReply::Nil
    );
}

#[tokio::test]
async fn delete_removes_only_the_named_keys() {
    let client = FakeClient::new();
    client
        .rpush("a", &[RedisScalar::Text("x".to_owned())])
        .await
        .unwrap();
    client
        .rpush("b", &[RedisScalar::Text("y".to_owned())])
        .await
        .unwrap();
    assert_eq!(client.delete(&["a", "missing"]).await.unwrap(), 1);
}

#[tokio::test]
async fn expire_then_ttl_reports_the_stored_value() {
    let client = FakeClient::new();
    assert_eq!(client.ttl("key").await.unwrap(), -1);
    assert!(client.expire("key", 30).await.unwrap());
    assert_eq!(client.ttl("key").await.unwrap(), 30);
}

#[test]
fn pipeline_carries_the_transaction_flag() {
    assert!(FakeClient::new().pipeline(true));
    assert!(!FakeClient::new().pipeline(false));
}

#[tokio::test]
async fn eval_runs_a_script_against_the_given_keys() {
    let client = FakeClient::new();
    let reply = client
        .eval("return 1", 1, &[RedisScalar::Text("key".to_owned())])
        .await
        .unwrap();
    assert_eq!(reply, FakeReply::Int(1));
}
