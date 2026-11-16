use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::{
    ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::openai_base_config::Object;

use super::fixture::{container_config, object};
use crate::openai_resource_dispatch::containers::{Call, Dispatch, Outcome};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://api.containers.test/v1";

async fn run(
    call: Call<'_>,
    litellm_params: &Object,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let config = container_config();
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params,
        headers: &headers,
    };
    let operation = OpenAiResourceOperation::Containers(Box::new(Dispatch {
        config: &config,
        context,
        call,
    }));
    dispatch(operation).await
}

async fn request_row(call: Call<'_>, url: &str, query: &str) {
    let result = run(call, &object("{}")).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Request((sent_url, sent_query)))) = result
    else {
        panic!("the request transform returns its url and query");
    };
    assert_eq!(sent_url, url);
    assert_eq!(sent_query, object(query));
}

async fn create_row() {
    let params = object(r#"{"file_ids":["file-1"]}"#);
    let call = Call::Create {
        name: "box",
        params: &params,
    };
    let result = run(call, &params).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Create(body))) = result else {
        panic!("the create request transform returns its body");
    };
    assert_eq!(body, object(r#"{"name":"box","file_ids":["file-1"]}"#));
}

async fn list_rows() {
    let extra = object(r#"{"filter":"mine"}"#);
    let full = ContainerListQuery {
        after: Some("cntr_0"),
        limit: Some(2),
        order: Some("desc"),
        extra_query: Some(&extra),
    };
    let call = Call::List {
        api_base: API_BASE,
        query: full,
    };
    let sent = r#"{"after":"cntr_0","limit":"2","order":"desc","filter":"mine"}"#;
    request_row(call, API_BASE, sent).await;
    let limited = ContainerListQuery {
        after: None,
        limit: Some(1),
        order: None,
        extra_query: None,
    };
    let files = Call::FileList {
        container_id: "cntr_1",
        api_base: API_BASE,
        query: limited,
    };
    let files_url = "https://api.containers.test/v1/cntr_1/files";
    request_row(files, files_url, r#"{"limit":"1"}"#).await;
}

async fn path_rows() {
    let container_url = "https://api.containers.test/v1/cntr_1";
    let retrieve = Call::Retrieve {
        id: "cntr_1",
        api_base: API_BASE,
    };
    request_row(retrieve, container_url, "{}").await;
    let delete = Call::Delete {
        id: "cntr_1",
        api_base: API_BASE,
    };
    request_row(delete, container_url, "{}").await;
    let content = Call::FileContent {
        container_id: "cntr_1",
        file_id: "cfile_1",
        api_base: API_BASE,
    };
    let content_url = "https://api.containers.test/v1/cntr_1/files/cfile_1/content";
    request_row(content, content_url, "{}").await;
}

async fn content_reply_row() {
    let raw = b"\x00\x01raw\xff";
    let result = run(Call::FileContentReply(raw), &object("{}")).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::FileContent(content))) = result else {
        panic!("the file content reply keeps its bytes");
    };
    assert_eq!(content.as_ref(), raw);
}

async fn error_row() {
    let call = Call::Retrieve {
        id: "..",
        api_base: API_BASE,
    };
    let result = run(call, &object("{}")).await;
    let Err(error) = result else {
        panic!("a dot path segment is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Containers(ContainerError::PathSegment { .. })
    ));
}

pub(crate) async fn check() {
    create_row().await;
    list_rows().await;
    path_rows().await;
    content_reply_row().await;
    error_row().await;
}
