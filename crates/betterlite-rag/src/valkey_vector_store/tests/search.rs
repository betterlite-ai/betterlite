use std::time::Duration;

use betterlite_provider::api::vector_store_contracts::config::SearchQuery;
use rstest::rstest;

use super::fakes::{
    ConnectCall, FakeConnector, FakeEmbedder, FakePort, PortLog, PortOutcome, document,
    empty_params, params, text_documents,
};
use crate::valkey_vector_store::error::ValkeyError;
use crate::valkey_vector_store::ports::{SocketTimeouts, ValkeyTimeout};
use crate::valkey_vector_store::query::ValkeyKnnQuery;
use crate::valkey_vector_store::{ValkeySearchRequest, search};

const MODEL: &str = "openai/text-embedding-3-small";

fn embedding_params() -> String {
    format!(
        r#"{{"valkey_host":"valkey.example.com","valkey_port":6380,"valkey_password":"p@ss","valkey_ssl":true,"litellm_embedding_model":"{MODEL}"}}"#
    )
}

fn knn_query() -> ValkeyKnnQuery {
    ValkeyKnnQuery {
        expression: "*=>[KNN 10 @embedding $vec AS vector_distance]".to_owned(),
        return_fields: vec!["text".to_owned(), "vector_distance".to_owned()],
        offset: 0,
        count: 10,
        dialect: 2,
    }
}

#[tokio::test]
async fn search_builds_the_url_sends_the_knn_query_and_closes_the_port() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![0.5, -1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(text_documents()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::Uniform(Duration::from_secs(7)),
    };
    let response = search(&request, &embedder, &connector, None)
        .await
        .expect("the search");
    assert_eq!(response.search_query.as_deref(), Some("find me"));
    assert_eq!(
        connector.calls(),
        vec![ConnectCall {
            url: "rediss://:p%40ss@valkey.example.com:6380".to_owned(),
            timeouts: SocketTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_secs(7),
            },
        }]
    );
    let calls = connector.log.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].index, "vs_index");
    assert_eq!(calls[0].query, knn_query());
    assert_eq!(calls[0].params["vec"], vec![0, 0, 0, 63, 0, 0, 128, 191]);
    assert!(connector.log.closed());
    let embedding_calls = embedder.calls();
    assert_eq!(embedding_calls.len(), 1);
    assert_eq!(embedding_calls[0].model, MODEL);
    assert_eq!(embedding_calls[0].text, "find me");
    assert!(embedding_calls[0].configuration.is_empty());
}

#[tokio::test]
async fn search_passes_the_embedding_configuration() {
    let litellm_params = params(&format!(
        r#"{{"valkey_host":"h","litellm_embedding_model":"{MODEL}","litellm_embedding_config":{{"dimensions":8}}}}"#
    ));
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    search(&request, &embedder, &connector, None)
        .await
        .expect("the search");
    let embedding_calls = embedder.calls();
    assert_eq!(
        embedding_calls[0].configuration,
        params(r#"{"dimensions":8}"#)
    );
}

#[tokio::test]
async fn search_joins_a_query_list_before_embedding() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Sequence(vec!["one".to_owned(), "two".to_owned()]);
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    search(&request, &embedder, &connector, None)
        .await
        .expect("the search");
    assert_eq!(embedder.calls()[0].text, "one two");
}

#[tokio::test]
async fn search_packs_the_first_vector_only() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0], vec![2.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    search(&request, &embedder, &connector, None)
        .await
        .expect("the search");
    assert_eq!(connector.log.calls()[0].params["vec"], vec![0, 0, 128, 63]);
}

#[tokio::test]
async fn search_closes_the_port_after_a_search_error() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Failure);
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the search failure");
    assert!(matches!(error, ValkeyError::UnexpectedReply { .. }));
    assert!(connector.log.closed());
}

#[tokio::test]
async fn search_closes_the_port_after_a_reply_error() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let broken = vec![document("doc", &[("text", "no distance")])];
    let connector = FakeConnector::new(PortOutcome::Documents(broken));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the reply failure");
    assert!(matches!(error, ValkeyError::MissingDistance { .. }));
    assert!(connector.log.closed());
}

#[tokio::test]
async fn search_uses_a_ready_port_without_a_url_and_leaves_it_open() {
    let litellm_params = params(&format!(r#"{{"litellm_embedding_model":"{MODEL}"}}"#));
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Failure);
    let port = FakePort {
        log: PortLog::default(),
        outcome: PortOutcome::Documents(text_documents()),
    };
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::Uniform(Duration::from_secs(7)),
    };
    let response = search(&request, &embedder, &connector, Some(&port))
        .await
        .expect("the search");
    assert_eq!(response.search_query.as_deref(), Some("find me"));
    assert_eq!(port.log.calls().len(), 1);
    assert!(connector.calls().is_empty());
    assert!(!port.log.closed());
}

#[tokio::test]
async fn search_requires_the_host_after_embedding() {
    let litellm_params = params(&format!(r#"{{"litellm_embedding_model":"{MODEL}"}}"#));
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the missing host");
    assert!(matches!(error, ValkeyError::HostRequired));
    assert_eq!(embedder.calls().len(), 1);
    assert!(connector.calls().is_empty());
}

#[tokio::test]
async fn search_rejects_filters_before_embedding() {
    let litellm_params = empty_params();
    let optional_params = params(r#"{"filters":{"key":"tag"}}"#);
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the filters");
    assert!(matches!(error, ValkeyError::FiltersUnsupported));
    assert!(embedder.calls().is_empty());
}

#[tokio::test]
async fn search_requires_the_embedding_model_before_embedding() {
    let litellm_params = params(r#"{"valkey_host":"h"}"#);
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the missing model");
    assert!(matches!(error, ValkeyError::EmbeddingModelRequired));
    assert!(embedder.calls().is_empty());
}

#[tokio::test]
async fn search_reports_an_embedding_failure_without_connecting() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::failing();
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the embedding failure");
    assert!(matches!(error, ValkeyError::Embedding { .. }));
    assert!(connector.calls().is_empty());
}

#[tokio::test]
async fn search_reports_an_empty_embedding_response_without_connecting() {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(Vec::new());
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout: ValkeyTimeout::None,
    };
    let error = search(&request, &embedder, &connector, None)
        .await
        .expect_err("the empty response");
    assert!(matches!(error, ValkeyError::NoEmbedding));
    assert!(connector.calls().is_empty());
}

#[rstest]
#[case::none(ValkeyTimeout::None, Duration::from_secs(5), Duration::from_secs(30))]
#[case::uniform_capped_connect(
    ValkeyTimeout::Uniform(Duration::from_secs(7)),
    Duration::from_secs(5),
    Duration::from_secs(7)
)]
#[case::uniform_below_the_cap(
    ValkeyTimeout::Uniform(Duration::from_secs(2)),
    Duration::from_secs(2),
    Duration::from_secs(2)
)]
#[case::connect_read(
    ValkeyTimeout::ConnectRead {
        connect: Some(Duration::from_secs(3)),
        read: None
    },
    Duration::from_secs(3),
    Duration::from_secs(30)
)]
#[case::read_only(
    ValkeyTimeout::ConnectRead {
        connect: None,
        read: Some(Duration::from_secs(12))
    },
    Duration::from_secs(5),
    Duration::from_secs(12)
)]
#[tokio::test]
async fn search_passes_the_socket_timeouts(
    #[case] timeout: ValkeyTimeout,
    #[case] connect: Duration,
    #[case] read: Duration,
) {
    let litellm_params = params(&embedding_params());
    let optional_params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let embedder = FakeEmbedder::answering(vec![vec![1.0]]);
    let connector = FakeConnector::new(PortOutcome::Documents(Vec::new()));
    let request = ValkeySearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        timeout,
    };
    search(&request, &embedder, &connector, None)
        .await
        .expect("the search");
    assert_eq!(
        connector.calls()[0].timeouts,
        SocketTimeouts { connect, read }
    );
}
