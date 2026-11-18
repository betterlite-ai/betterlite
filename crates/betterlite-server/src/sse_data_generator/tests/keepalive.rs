#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use tokio::time::timeout;

use super::super::types::SseDataGenerator;
use super::{
    TestRouter, body_of, chat_chunk, chunk_frame, count_prefix, expected_chunk, object,
    plain_generator, quiet_source, recorded, recording_generator, request, with_hidden,
};
use crate::keepalive_interval::PingSetting;
use crate::stream_keepalive::KeepaliveRouter;

fn fast_hidden() -> JsonObject {
    object(&[("model_id", Json::Text(String::from("fast")))])
}

fn router_generator() -> SseDataGenerator<String> {
    let mut generator = plain_generator();
    let router: Arc<dyn KeepaliveRouter> = Arc::new(TestRouter);
    generator.router = Some(router);
    generator
}

#[tokio::test(start_paused = true)]
async fn router_pings_between_chunks() {
    let generator = router_generator();
    let hidden = fast_hidden();
    let item = with_hidden(chat_chunk("x"), hidden.clone());
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(item)], hidden),
        request(&[]),
        false,
    );
    let first = body.next().await.expect("chunk").expect("frame");
    assert_eq!(first, Bytes::from(chunk_frame(&expected_chunk("x", None))));
    let ping = timeout(Duration::from_secs(3), body.next())
        .await
        .expect("ping deadline")
        .expect("ping")
        .expect("frame");
    assert_eq!(ping, Bytes::from_static(b": ping\n\n"));
}

#[tokio::test(start_paused = true)]
async fn global_interval_pings() {
    let mut generator = plain_generator();
    generator.global_keepalive = Some(PingSetting::Seconds(2.0));
    let mut body = body_of(
        &generator,
        quiet_source(Vec::new(), JsonObject::new()),
        request(&[]),
        false,
    );
    let ping = timeout(Duration::from_secs(3), body.next())
        .await
        .expect("ping deadline")
        .expect("ping")
        .expect("frame");
    assert_eq!(ping, Bytes::from_static(b": ping\n\n"));
}

#[tokio::test(start_paused = true)]
async fn disabled_keepalive_is_quiet() {
    let generator = plain_generator();
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[]),
        false,
    );
    let first = body.next().await.expect("chunk");
    first.expect("successful stream frame");
    timeout(Duration::from_secs(30), body.next())
        .await
        .expect_err("stream stays pending");
}

#[tokio::test(start_paused = true)]
async fn fallback_turns_pings_on() {
    let generator = router_generator();
    let item = with_hidden(chat_chunk("x"), fast_hidden());
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(item)], JsonObject::new()),
        request(&[]),
        false,
    );
    let first = body.next().await.expect("chunk");
    first.expect("successful stream frame");
    let ping = timeout(Duration::from_secs(3), body.next())
        .await
        .expect("ping deadline")
        .expect("ping")
        .expect("frame");
    assert_eq!(ping, Bytes::from_static(b": ping\n\n"));
}

#[tokio::test(start_paused = true)]
async fn explicit_disable_stops_pings() {
    let generator = router_generator();
    let hidden = object(&[("model_id", Json::Text(String::from("slow")))]);
    let item = with_hidden(chat_chunk("x"), hidden.clone());
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(item)], hidden),
        request(&[]),
        false,
    );
    let first = body.next().await.expect("chunk");
    first.expect("successful stream frame");
    timeout(Duration::from_secs(30), body.next())
        .await
        .expect_err("stream stays pending");
}

#[tokio::test(start_paused = true)]
async fn ping_does_not_reach_chunk_hook() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (mut generator, hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    hooks.set_per_chunk(true);
    let router: Arc<dyn KeepaliveRouter> = Arc::new(TestRouter);
    generator.router = Some(router);
    let hidden = fast_hidden();
    let item = with_hidden(chat_chunk("x"), hidden.clone());
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(item)], hidden),
        request(&[]),
        false,
    );
    let chunk = body.next().await.expect("chunk");
    chunk.expect("successful stream frame");
    let ping = timeout(Duration::from_secs(3), body.next())
        .await
        .expect("ping deadline")
        .expect("ping");
    ping.expect("successful stream frame");
    assert_eq!(count_prefix(&recorded(&events), "chunk:"), 1);
}
