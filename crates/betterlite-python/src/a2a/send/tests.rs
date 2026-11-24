#![cfg(test)]

use pyo3::prelude::*;

use super::send;
use crate::a2a::convert::Call;

#[tokio::test]
async fn send_rejects_a_call_without_a_request() {
    Python::initialize();
    Box::pin(send(None, Call::default())).await.unwrap_err();
}
