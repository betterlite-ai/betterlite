use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::openai_base_config::Object;

use super::fixture::{container_config, object};
use crate::openai_resource_dispatch::containers::{
    Call, Dispatch, Outcome, ReplyCall, RequestCall,
};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://api.containers.test/v1";

async fn run(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::Containers(Box::new(Dispatch {
        config,
        context,
        call,
    }));
    dispatch(operation).await
}

fn container_body() -> Vec<u8> {
    br#"{"id":"cntr_1","object":"container","created_at":1700000000,"status":"running","name":"box"}"#
        .to_vec()
}

fn request_result(result: Result<OpenAiResourceResult, OpenAiResourceError>) -> (String, Object) {
    let Ok(OpenAiResourceResult::Containers(Outcome::Request { url, query })) = result else {
        panic!("the request transform returns its url and query");
    };
    (url, query)
}

async fn create_rows() {
    let config = container_config();
    let params = object(r#"{"file_ids":["file-1"]}"#);
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::Create {
            name: "box",
            params: &params,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Create(body))) = request else {
        panic!("the create request transform returns its body");
    };
    assert_eq!(body, object(r#"{"name":"box","file_ids":["file-1"]}"#));
    let reply = run(
        &config,
        context,
        Call::Reply(ReplyCall::Create(&container_body())),
    )
    .await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Container(container))) = reply else {
        panic!("the create reply decodes to a container");
    };
    assert_eq!(container.id, "cntr_1");
    assert_eq!(container.name.as_deref(), Some("box"));
}

async fn list_rows() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let extra = object(r#"{"filter":"mine"}"#);
    let query = ContainerListQuery {
        after: Some("cntr_0"),
        limit: Some(2),
        order: Some("desc"),
        extra_query: Some(&extra),
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::List {
            api_base: API_BASE,
            query,
        }),
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, API_BASE);
    assert_eq!(
        query,
        object(r#"{"after":"cntr_0","limit":"2","order":"desc","filter":"mine"}"#)
    );
    let body = br#"{"object":"list","data":[{"id":"cntr_1","object":"container","created_at":1,"status":"running"}],"has_more":false,"first_id":"cntr_1","last_id":"cntr_1"}"#;
    let reply = run(&config, context, Call::Reply(ReplyCall::List(body))).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::List(list))) = reply else {
        panic!("the list reply decodes to a container page");
    };
    assert_eq!(list.data.len(), 1);
    assert!(!list.has_more);
}

async fn retrieve_rows() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::Retrieve {
            container_id: "cntr_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1");
    assert_eq!(query, object("{}"));
    let reply = run(
        &config,
        context,
        Call::Reply(ReplyCall::Retrieve(&container_body())),
    )
    .await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Container(container))) = reply else {
        panic!("the retrieve reply decodes to a container");
    };
    assert_eq!(container.status, "running");
}

async fn delete_rows() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::Delete {
            container_id: "cntr_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1");
    assert_eq!(query, object("{}"));
    let body = br#"{"id":"cntr_1","object":"container.deleted","deleted":true}"#;
    let reply = run(&config, context, Call::Reply(ReplyCall::Delete(body))).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Deleted(deleted))) = reply else {
        panic!("the delete reply decodes to a deleted container");
    };
    assert!(deleted.deleted);
}

async fn file_list_rows() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let query = ContainerListQuery {
        after: None,
        limit: Some(1),
        order: None,
        extra_query: None,
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::FileList {
            container_id: "cntr_1",
            api_base: API_BASE,
            query,
        }),
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1/files");
    assert_eq!(query, object(r#"{"limit":"1"}"#));
    let body = br#"{"object":"list","data":[{"id":"cfile_1","object":"container.file","container_id":"cntr_1","created_at":2,"path":"/mnt/x.txt","source":"assistant"}],"has_more":false,"first_id":"cfile_1","last_id":"cfile_1"}"#;
    let reply = run(&config, context, Call::Reply(ReplyCall::FileList(body))).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::FileList(list))) = reply else {
        panic!("the file list reply decodes to a container file page");
    };
    assert_eq!(list.data[0].path, "/mnt/x.txt");
    assert_eq!(list.data[0].container_id, "cntr_1");
}

async fn file_content_rows() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Request(RequestCall::FileContent {
            container_id: "cntr_1",
            file_id: "cfile_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(
        url,
        "https://api.containers.test/v1/cntr_1/files/cfile_1/content"
    );
    assert_eq!(query, object("{}"));
    let reply = run(
        &config,
        context,
        Call::Reply(ReplyCall::FileContent(b"\x00\x01raw\xff")),
    )
    .await;
    let Ok(OpenAiResourceResult::Containers(Outcome::FileContent(content))) = reply else {
        panic!("the file content reply keeps its bytes");
    };
    assert_eq!(content.as_ref(), b"\x00\x01raw\xff");
}

async fn error_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let result = run(
        &config,
        context,
        Call::Reply(ReplyCall::Retrieve(b"not a container")),
    )
    .await;
    let Err(error) = result else {
        panic!("a malformed container reply is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Containers(ContainerError::Decode(_))
    ));
}

pub(crate) async fn check() {
    create_rows().await;
    list_rows().await;
    retrieve_rows().await;
    delete_rows().await;
    file_list_rows().await;
    file_content_rows().await;
    error_row().await;
}
