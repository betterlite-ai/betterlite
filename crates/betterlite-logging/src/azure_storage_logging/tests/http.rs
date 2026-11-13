#![cfg(test)]

use super::support::{ENTRA, Reply, fixture, payload};

fn token_reply() -> Reply {
    Reply {
        status: 200,
        body: r#"{"access_token": "tok-1"}"#,
    }
}

#[tokio::test]
async fn the_token_path_creates_appends_and_flushes_one_file() {
    let fixture = fixture(&ENTRA, &[token_reply(), Reply::OK, Reply::OK, Reply::OK]);
    let payload = payload("resp_YWJj/ZA==");
    fixture
        .logger
        .upload_payload(&payload)
        .await
        .expect("the scripted storage accepts the upload");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 4);
    let create = calls.get(1).expect("the create call");
    assert_eq!(create.method, "PUT");
    assert_eq!(
        create.url,
        "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?resource=file"
    );
    assert_eq!(create.header("x-ms-version"), Some("2019-07-07"));
    assert_eq!(create.header("Content-Length"), Some("0"));
    assert_eq!(create.header("Authorization"), Some("Bearer tok-1"));
    let append = calls.get(2).expect("the append call");
    assert_eq!(append.method, "PATCH");
    assert_eq!(
        append.url,
        "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?action=append&position=0"
    );
    assert_eq!(append.header("Content-Type"), Some("application/json"));
    assert_eq!(append.header("Authorization"), Some("Bearer tok-1"));
    let text = std::str::from_utf8(&append.body).expect("utf-8 json");
    assert!(text.ends_with('\n'));
    let body: serde_json::Value =
        serde_json::from_str(text.trim_end_matches('\n')).expect("json body");
    assert_eq!(
        body.get("id").and_then(serde_json::Value::as_str),
        Some("resp_YWJj/ZA==")
    );
    let flush = calls.get(3).expect("the flush call");
    assert_eq!(flush.method, "PATCH");
    assert_eq!(
        flush.url,
        format!(
            "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?action=flush&position={}",
            append.body.len()
        )
    );
    assert_eq!(flush.header("Content-Length"), Some("0"));
}

#[tokio::test]
async fn a_failed_create_stops_before_the_append() {
    let fixture = fixture(
        &ENTRA,
        &[
            token_reply(),
            Reply {
                status: 500,
                body: "",
            },
        ],
    );
    let payload = payload("id-1");
    let error = fixture
        .logger
        .upload_payload(&payload)
        .await
        .expect_err("a 500 is an upload failure");
    assert!(error.to_string().contains("500"));
    assert_eq!(fixture.transport.calls().len(), 2);
}

#[tokio::test]
async fn each_upload_writes_its_own_file() {
    let fixture = fixture(
        &ENTRA,
        &[
            token_reply(),
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
        ],
    );
    fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect("first upload");
    fixture
        .logger
        .upload_payload(&payload("id-2"))
        .await
        .expect("second upload");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 7);
    assert!(calls.get(1).expect("create").url.contains("/id-1.json?"));
    assert!(calls.get(4).expect("create").url.contains("/id-2.json?"));
}
