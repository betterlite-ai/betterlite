use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest;
use tokio::time::timeout;

use super::*;
use crate::embedding_rerank_providers::EmbeddingRerankRequest;
use crate::operation::{ChatOperation, EmbeddingOperation, OperationRequest};
use crate::prepared::PreparedRequest;
#[test]
fn the_adapter_refuses_a_model_without_a_family_slug() {
    let request = OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("gpt-4o"),
        request: ChatRequest::parse(br#"{"messages":[]}"#).unwrap(),
        control: ControlOptions::default(),
    }));
    let refused = EmbeddingRerank.prepare(
        &request,
        &deployment("gpt-4o", "https://host", None),
        &catalog(),
    );
    assert!(matches!(
        refused,
        Err(OperationError::UnavailableAdapter { .. })
    ));
    let crossed = EmbeddingRerank.prepare(
        &request,
        &deployment("openai/gpt-4o", "https://host", None),
        &catalog(),
    );
    assert!(matches!(
        crossed,
        Err(OperationError::UnavailableAdapter { .. })
    ));
}

#[test]
fn the_adapter_refuses_its_own_unit_request() {
    let request = OperationRequest::EmbeddingRerank(EmbeddingRerankRequest);
    let refused = EmbeddingRerank.prepare(
        &request,
        &deployment("cohere/embed-v4.0", "https://host", None),
        &catalog(),
    );
    assert!(matches!(
        refused,
        Err(OperationError::UnavailableAdapter { .. })
    ));
}

#[test]
fn a_family_preparation_failure_reaches_the_caller_as_an_unsupported_operation() {
    let embedding = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("cohere/embed-v4.0"),
        request: serde_json::from_str(r#"{"input":"hi"}"#).unwrap(),
    }));
    let mut deployed = deployment("cohere/embed-v4.0", "https://host", Some("key"));
    deployed.api_base = String::from("not a url");
    let refused = EmbeddingRerank.prepare(&embedding, &deployed, &catalog());
    assert!(matches!(
        refused,
        Err(OperationError::UnsupportedOperation { provider, operation })
            if provider == "embedding_rerank" && operation == "prepare"
    ));
}

#[test]
fn the_adapter_prepares_embeddings_from_the_deployment() {
    let embedding = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("cohere/embed-v4.0"),
        request: serde_json::from_str(r#"{"input":"hi"}"#).unwrap(),
    }));
    let prepared = EmbeddingRerank
        .prepare(
            &embedding,
            &deployment("cohere/embed-v4.0", "https://host/v2/embed", Some("key")),
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::EmbeddingRerank(call) = prepared else {
        panic!("the embedding prepared call");
    };
    assert_eq!(url_of(&call), "https://host/v2/embed");
}

#[tokio::test]
async fn the_adapter_sends_and_decodes_a_family_chat() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let deployed = deployment("cohere/v2/command-r", &base, Some("key"));
    let request = OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("cohere/v2/command-r"),
        request: ChatRequest::parse(br#"{"messages":[{"role":"user","content":"hi"}]}"#).unwrap(),
        control: ControlOptions::default(),
    }));
    let prepared = EmbeddingRerank
        .prepare(&request, &deployed, &catalog())
        .unwrap();
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(serve(&listener, 200, V2_CHAT_REPLY.as_bytes()), async {
            let upstream = EmbeddingRerank
                .execute(&client(), &prepared, &deployed)
                .await
                .unwrap();
            EmbeddingRerank.decode(&prepared, upstream)
        })
    })
    .await
    .unwrap();
    let lowered = sent.to_ascii_lowercase();
    assert!(sent.starts_with("POST / HTTP/1.1"));
    assert!(lowered.contains("authorization: bearer key"));
    assert!(sent.contains(r#""model":"command-r""#));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::CohereV2Chat(reply) = result else {
        panic!("the v2 chat reply");
    };
    assert_eq!(reply.message.content.as_deref(), Some("hello"));
}

#[tokio::test]
async fn a_cohere_rerank_call_is_sent_and_projected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v2/rerank", listener.local_addr().unwrap());
    let mut fixture = RerankFixture::new(ProviderId::Cohere, "rerank-v3.5");
    fixture.setup.api_base = Some(base);
    let prepared = PreparedRequest::EmbeddingRerank(fixture.prepared());
    let deployed = deployment("cohere/rerank-v3.5", "https://host", Some("key"));
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(serve(&listener, 200, RERANK_REPLY.as_bytes()), async {
            let upstream = EmbeddingRerank
                .execute(&client(), &prepared, &deployed)
                .await
                .unwrap();
            EmbeddingRerank.decode(&prepared, upstream)
        })
    })
    .await
    .unwrap();
    assert!(sent.starts_with("POST /v2/rerank HTTP/1.1"));
    assert!(sent.contains(r#""query":"what""#));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::Rerank(reply) = result else {
        panic!("the rerank answer");
    };
    let results = reply.results.unwrap();
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
}

#[tokio::test]
async fn a_jina_rerank_call_is_sent_and_projected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut fixture = RerankFixture::new(ProviderId::JinaAi, "jina-reranker-v2");
    fixture.setup.api_base = Some(base);
    let prepared = PreparedRequest::EmbeddingRerank(fixture.prepared());
    let deployed = deployment("jina_ai/jina-reranker-v2", "https://host", Some("key"));
    let reply = r#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5,"document":{"text":"doc"}}],"usage":{"total_tokens":2}}"#;
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(serve(&listener, 200, reply.as_bytes()), async {
            let upstream = EmbeddingRerank
                .execute(&client(), &prepared, &deployed)
                .await
                .unwrap();
            EmbeddingRerank.decode(&prepared, upstream)
        })
    })
    .await
    .unwrap();
    assert!(sent.starts_with("POST /v1/rerank HTTP/1.1"));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::Rerank(decoded) = result else {
        panic!("the rerank answer");
    };
    let results = decoded.results.unwrap();
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
}

#[tokio::test]
async fn an_infinity_rerank_call_is_sent_and_projected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut fixture = RerankFixture::new(ProviderId::Infinity, "infinity-rerank");
    fixture.setup.api_base = Some(base);
    let prepared = PreparedRequest::EmbeddingRerank(fixture.prepared());
    let deployed = deployment("infinity/infinity-rerank", "https://host", Some("key"));
    let reply = r#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5}]}"#;
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(serve(&listener, 200, reply.as_bytes()), async {
            let upstream = EmbeddingRerank
                .execute(&client(), &prepared, &deployed)
                .await
                .unwrap();
            EmbeddingRerank.decode(&prepared, upstream)
        })
    })
    .await
    .unwrap();
    assert!(sent.starts_with("POST /rerank HTTP/1.1"));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::Rerank(decoded) = result else {
        panic!("the rerank answer");
    };
    assert_eq!(decoded.id.as_deref(), Some("r-1"));
}

#[tokio::test]
async fn a_voyage_rerank_call_is_sent_and_projected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut fixture = RerankFixture::new(ProviderId::Voyage, "rerank-2");
    fixture.setup.api_base = Some(base);
    let prepared = PreparedRequest::EmbeddingRerank(fixture.prepared());
    let deployed = deployment("voyage/rerank-2", "https://host", Some("key"));
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(
            serve(&listener, 200, VOYAGE_RERANK_REPLY.as_bytes()),
            async {
                let upstream = EmbeddingRerank
                    .execute(&client(), &prepared, &deployed)
                    .await
                    .unwrap();
                EmbeddingRerank.decode(&prepared, upstream)
            }
        )
    })
    .await
    .unwrap();
    assert!(sent.starts_with("POST /v1/rerank HTTP/1.1"));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::Rerank(reply) = result else {
        panic!("the rerank answer");
    };
    let results = reply.results.unwrap();
    assert_eq!(results[0].document.as_ref().unwrap().text, "doc");
}

#[tokio::test]
async fn a_cohere_embedding_call_is_sent_and_decoded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v2/embed", listener.local_addr().unwrap());
    let mut fixture = EmbeddingFixture::new(ProviderId::Cohere, "embed-v4.0");
    fixture.setup.api_base = Some(base);
    let prepared = PreparedRequest::EmbeddingRerank(fixture.prepared());
    let deployed = deployment("cohere/embed-v4.0", "https://host", Some("key"));
    let (sent, answer) = timeout(EXCHANGE, async {
        tokio::join!(
            serve(&listener, 200, COHERE_EMBEDDING_REPLY.as_bytes()),
            async {
                let upstream = EmbeddingRerank
                    .execute(&client(), &prepared, &deployed)
                    .await
                    .unwrap();
                EmbeddingRerank.decode(&prepared, upstream)
            }
        )
    })
    .await
    .unwrap();
    assert!(sent.starts_with("POST /v2/embed HTTP/1.1"));
    assert!(sent.contains(r#""texts":["hi"]"#));
    let result = embedding_result(answer.unwrap());
    let EmbeddingRerankResult::CohereEmbedding(reply) = result else {
        panic!("the cohere embedding reply");
    };
    assert_eq!(reply.usage.prompt_tokens, 2);
}
