#![cfg(test)]

mod types {
    use std::fs;
    use std::path::PathBuf;

    use secrecy::ExposeSecret as _;

    use crate::rag::document::chunking::{
        ChunkingType, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE, RAGChunkingStrategy,
        StaticChunkingStrategy, WhiteSpaceChunkingConfig,
    };
    use crate::rag::document::ingest::{
        BedrockVectorStoreOptions, OpenAIVectorStoreOptions, RAGIngestEmbeddingOptions,
        RAGIngestOCROptions, RAGIngestOptions, RAGIngestStatus, RAGIngestVectorStoreOptions,
        S3VectorsDistanceMetric, S3VectorsVectorStoreOptions, VertexAIVectorStoreOptions,
    };
    use crate::rag::document::{INGESTION_PROVIDERS, IngestionProvider, RAGIngestResponse};

    #[test]
    fn chunking_strategy_fields() {
        let strategy: RAGChunkingStrategy = serde_json::from_str(
            r#"{"chunk_size": 500, "chunk_overlap": 50, "separators": ["\n\n", "\n"]}"#,
        )
        .expect("every field here matches the struct");
        assert_eq!(strategy.chunk_size, Some(500));
        assert_eq!(strategy.chunk_overlap, Some(50));
        assert_eq!(
            strategy.separators,
            vec!["\n\n".to_owned(), "\n".to_owned()]
        );
    }

    #[test]
    fn chunking_strategy_leaves_unsent_sizes_empty() {
        let strategy: RAGChunkingStrategy =
            serde_json::from_str("{}").expect("every field has a default");
        assert_eq!(DEFAULT_CHUNK_SIZE, 1000);
        assert_eq!(DEFAULT_CHUNK_OVERLAP, 200);
        assert_eq!(strategy.chunk_size, None);
        assert_eq!(strategy.chunk_overlap, None);
        assert_eq!(
            strategy.separators,
            vec![
                "\n\n".to_owned(),
                "\n".to_owned(),
                " ".to_owned(),
                String::new()
            ]
        );
        assert_eq!(strategy, RAGChunkingStrategy::default());
    }

    #[test]
    fn chunking_strategy_null_separators_also_defaults() {
        let strategy: RAGChunkingStrategy =
            serde_json::from_str(r#"{"separators": null}"#).expect("null falls back to default");
        assert_eq!(
            strategy.separators,
            RAGChunkingStrategy::default().separators
        );
    }

    #[test]
    fn chunking_strategy_explicit_empty_separators_is_used_as_given() {
        let strategy: RAGChunkingStrategy =
            serde_json::from_str(r#"{"separators": []}"#).expect("an empty list is valid");
        assert_eq!(strategy.separators, Vec::<String>::new());
    }

    #[test]
    fn chunking_strategy_rejects_an_unknown_field() {
        let error =
            serde_json::from_str::<RAGChunkingStrategy>(r#"{"chunk_bogus": 500}"#).unwrap_err();
        assert!(error.to_string().contains("chunk_bogus"));
    }

    #[test]
    fn ingest_options_rejects_an_unknown_field_nested_inside_a_vector_store_record() {
        let error = serde_json::from_str::<RAGIngestOptions>(
            r#"{
                "vector_store": {"custom_llm_provider": "openai", "api_kye": "sk-secret"}
            }"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("api_kye"));
    }

    #[rstest::rstest]
    #[case::absent(r#"{"vector_store":{"vector_store_id":"vs_1"}}"#)]
    #[case::null(r#"{"vector_store":{"custom_llm_provider":null,"vector_store_id":"vs_1"}}"#)]
    fn vector_store_options_default_null_provider(#[case] input: &str) {
        let options: RAGIngestOptions = serde_json::from_str(input).unwrap();
        assert!(matches!(
            options.vector_store,
            Some(RAGIngestVectorStoreOptions::Openai(_))
        ));
    }

    #[test]
    fn vector_store_options_without_a_provider_parse_as_openai() {
        let options: RAGIngestOptions =
            serde_json::from_str(r#"{"vector_store":{"vector_store_id":"vs_1"}}"#)
                .expect("provider defaults to openai");
        assert!(matches!(
            options.vector_store,
            Some(RAGIngestVectorStoreOptions::Openai(_))
        ));
    }

    #[test]
    fn vector_store_options_accept_the_gemini_provider() {
        let options: RAGIngestVectorStoreOptions = serde_json::from_str(r#"{"custom_llm_provider":"gemini","api_key":"gemini-secret","api_base":"https://gemini.test","vector_store_id":"store","litellm_credential_name":"credential","custom_metadata":[{"key":"x","string_value":"y"}]}"#).unwrap();
        assert!(!format!("{options:?}").contains("gemini-secret"));
        let RAGIngestVectorStoreOptions::Gemini(record) = options else {
            panic!("Gemini record")
        };
        assert_eq!(record.api_key.unwrap().expose_secret(), "gemini-secret");
        assert_eq!(record.api_base.as_deref(), Some("https://gemini.test"));
        assert_eq!(record.vector_store_id.as_deref(), Some("store"));
        assert_eq!(
            record.litellm_credential_name.as_deref(),
            Some("credential")
        );
        assert_eq!(
            record.custom_metadata.unwrap().to_string(),
            r#"[{"key":"x","string_value":"y"}]"#
        );
    }

    #[rstest::rstest]
    #[case::bedrock(
        "bedrock",
        "knowledge_base_id",
        r#""kb""#,
        r#"knowledge_base_id: Some("kb")"#
    )]
    #[case::description(
        "vertex_ai",
        "description",
        r#""desc""#,
        r#"description: Some("desc")"#
    )]
    #[case::vector_db_config(
        "vertex_ai",
        "vector_db_config",
        "{}",
        "vector_db_config: Some(Object {})"
    )]
    #[case::vertex_embedding(
        "vertex_ai",
        "embedding_model",
        r#""embed""#,
        r#"embedding_model: Some("embed")"#
    )]
    #[case::file_description(
        "vertex_ai",
        "file_description",
        r#""file""#,
        r#"file_description: Some("file")"#
    )]
    #[case::rate(
        "vertex_ai",
        "max_embedding_requests_per_min",
        "2",
        "max_embedding_requests_per_min: Some(2)"
    )]
    #[case::credential(
        "vertex_ai",
        "litellm_credential_name",
        r#""cred""#,
        r#"litellm_credential_name: Some("cred")"#
    )]
    #[case::project(
        "vertex_ai",
        "vertex_ai_project",
        r#""project""#,
        r#"vertex_ai_project: Some("project")"#
    )]
    #[case::location(
        "vertex_ai",
        "vertex_ai_location",
        r#""location""#,
        r#"vertex_ai_location: Some("location")"#
    )]
    #[case::credentials(
        "vertex_ai",
        "vertex_ai_credentials",
        r#""secret""#,
        "vertex_ai_credentials: Some("
    )]
    #[case::ssl_bool("s3_vectors", "ssl_verify", "false", "ssl_verify: Some(Bool(false))")]
    #[case::ssl_path(
        "s3_vectors",
        "ssl_verify",
        r#""ca.pem""#,
        r#"ssl_verify: Some(CaBundle("ca.pem"))"#
    )]
    #[case::s3_embedding(
        "s3_vectors",
        "embedding_model",
        r#""embed""#,
        r#"embedding_model: Some("embed")"#
    )]
    #[case::litellm_embedding(
        "s3_vectors",
        "litellm_embedding_model",
        r#""embed""#,
        r#"litellm_embedding_model: Some("embed")"#
    )]
    fn vector_store_options_accept_a_key_litellm_reads(
        #[case] provider: &str,
        #[case] key: &str,
        #[case] value: &str,
        #[case] expected: &str,
    ) {
        let input = format!(r#"{{"custom_llm_provider":"{provider}","{key}":{value}}}"#);
        let options: RAGIngestVectorStoreOptions = serde_json::from_str(&input).unwrap();
        assert!(format!("{options:?}").contains(expected));
        if key == "vertex_ai_credentials" {
            assert!(!format!("{options:?}").contains("secret"));
            let RAGIngestVectorStoreOptions::VertexAi(record) = options else {
                panic!("Vertex record")
            };
            assert_eq!(
                record.vertex_ai_credentials.unwrap().expose_secret(),
                "secret"
            );
        }
    }

    #[rstest::rstest]
    #[case::auto(r#"{"type":"auto"}"#, Some(ChunkingType::Auto), None, None)]
    #[case::static_tokens(r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#, Some(ChunkingType::Static), Some(StaticChunkingStrategy { max_chunk_size_tokens: Some(800), chunk_overlap_tokens: Some(400) }), None)]
    #[case::white_space(r#"{"white_space_config":{"max_tokens_per_chunk":800,"max_overlap_tokens":400}}"#, None, None, Some(WhiteSpaceChunkingConfig { max_tokens_per_chunk: Some(800), max_overlap_tokens: Some(400) }))]
    fn chunking_strategy_accepts_a_provider_chunking_key(
        #[case] input: &str,
        #[case] kind: Option<ChunkingType>,
        #[case] tokens: Option<StaticChunkingStrategy>,
        #[case] whitespace: Option<WhiteSpaceChunkingConfig>,
    ) {
        let strategy: RAGChunkingStrategy = serde_json::from_str(input).unwrap();
        assert_eq!(strategy.kind, kind);
        assert_eq!(strategy.static_tokens, tokens);
        assert_eq!(strategy.white_space_config, whitespace);
    }

    #[rstest::rstest]
    #[case::untagged(r#"{"vector_store":{"api_kye":"secret"}}"#, "api_kye")]
    #[case::unknown(r#"{"vector_store":{"custom_llm_provider":"bogus"}}"#, "bogus")]
    #[case::wrong_type(r#"{"vector_store":{"custom_llm_provider":42}}"#, "string")]
    fn vector_store_options_reject_invalid_records(#[case] input: &str, #[case] message: &str) {
        let error = serde_json::from_str::<RAGIngestOptions>(input).unwrap_err();
        assert!(error.to_string().contains(message));
    }

    #[test]
    fn ocr_and_embedding_options_hold_a_model_name() {
        let ocr: RAGIngestOCROptions =
            serde_json::from_str(r#"{"model": "mistral-ocr"}"#).expect("model is a string");
        assert_eq!(ocr.model, Some("mistral-ocr".to_owned()));

        let embedding: RAGIngestEmbeddingOptions =
            serde_json::from_str(r#"{"model": "text-embedding-3-small"}"#)
                .expect("model is a string");
        assert_eq!(embedding.model, Some("text-embedding-3-small".to_owned()));
    }

    #[test]
    fn openai_vector_store_options_holds_a_secret_api_key() {
        let options: OpenAIVectorStoreOptions = serde_json::from_str(
            r#"{
                "vector_store_id": "vs_1",
                "ttl_days": 7,
                "litellm_credential_name": "cred",
                "api_key": "sk-secret",
                "api_base": "https://api.openai.com/v1"
            }"#,
        )
        .expect("every field here matches the struct");
        assert_eq!(options.vector_store_id, Some("vs_1".to_owned()));
        assert_eq!(options.ttl_days, Some(7));
        assert_eq!(
            options.api_key.expect("api_key was set").expose_secret(),
            "sk-secret"
        );
        assert_eq!(
            options.api_base,
            Some("https://api.openai.com/v1".to_owned())
        );
    }

    #[test]
    fn bedrock_vector_store_options_holds_its_aws_credential_fields() {
        let options: BedrockVectorStoreOptions = serde_json::from_str(
            r#"{
                "vector_store_id": "vs_bedrock",
                "s3_bucket": "bucket",
                "s3_prefix": "prefix/",
                "embedding_model": "amazon.titan-embed-text-v2:0",
                "data_source_id": "ds_1",
                "wait_for_ingestion": true,
                "ingestion_timeout": 60,
                "aws_access_key_id": "AKIA",
                "aws_secret_access_key": "secret",
                "aws_session_token": "session",
                "aws_region_name": "us-east-1",
                "aws_role_name": "role",
                "aws_session_name": "session-name",
                "aws_profile_name": "profile",
                "aws_web_identity_token": "web-token",
                "aws_sts_endpoint": "https://sts.amazonaws.com",
                "aws_external_id": "external"
            }"#,
        )
        .expect("every field here matches the struct");
        assert_eq!(options.vector_store_id, Some("vs_bedrock".to_owned()));
        assert_bedrock_secrets_expose_their_values(&options);
    }

    #[test]
    fn bedrock_debug_output_redacts_every_secret_field() {
        let options: BedrockVectorStoreOptions = serde_json::from_str(
            r#"{
                "aws_access_key_id": "akia-value",
                "aws_secret_access_key": "sak-value",
                "aws_session_token": "sess-value",
                "aws_web_identity_token": "wit-value",
                "aws_sts_endpoint": "sts-value",
                "aws_external_id": "ext-value"
            }"#,
        )
        .expect("every field here matches the struct");
        let debug_output = format!("{options:?}");
        assert!(!debug_output.contains("akia-value"));
        assert!(!debug_output.contains("sak-value"));
        assert!(!debug_output.contains("sess-value"));
        assert!(!debug_output.contains("wit-value"));
        assert!(!debug_output.contains("sts-value"));
        assert!(!debug_output.contains("ext-value"));
    }

    fn assert_bedrock_secrets_expose_their_values(options: &BedrockVectorStoreOptions) {
        let checks: [(&str, Option<&secrecy::SecretString>, &str); 6] = [
            (
                "aws_access_key_id",
                options.aws_access_key_id.as_ref(),
                "AKIA",
            ),
            (
                "aws_secret_access_key",
                options.aws_secret_access_key.as_ref(),
                "secret",
            ),
            (
                "aws_session_token",
                options.aws_session_token.as_ref(),
                "session",
            ),
            (
                "aws_web_identity_token",
                options.aws_web_identity_token.as_ref(),
                "web-token",
            ),
            (
                "aws_sts_endpoint",
                options.aws_sts_endpoint.as_ref(),
                "https://sts.amazonaws.com",
            ),
            (
                "aws_external_id",
                options.aws_external_id.as_ref(),
                "external",
            ),
        ];
        for (field_name, secret, expected) in checks {
            let value = secret.unwrap_or_else(|| panic!("{field_name} was set"));
            assert_eq!(value.expose_secret(), expected);
        }
    }

    #[test]
    fn vertex_ai_vector_store_options_parse_without_a_vector_store_id() {
        let options: VertexAIVectorStoreOptions = serde_json::from_str(
            r#"{
                "vector_store_id": "vs_vertex",
                "vertex_project": "my-project",
                "vertex_location": "us-central1",
                "vertex_credentials": "{\"private_key\": \"shh\"}",
                "gcs_bucket": "bucket",
                "wait_for_import": false,
                "import_timeout": 30
            }"#,
        )
        .expect("provider fields parse");
        assert_eq!(options.vector_store_id, Some("vs_vertex".to_owned()));
        assert_eq!(options.vertex_project, Some("my-project".to_owned()));
        assert_eq!(
            options
                .vertex_credentials
                .as_ref()
                .expect("vertex_credentials was set")
                .expose_secret(),
            "{\"private_key\": \"shh\"}"
        );

        let missing_id = serde_json::from_str::<VertexAIVectorStoreOptions>(
            r#"{"vertex_project": "my-project"}"#,
        )
        .expect("corpus creation can omit the id");
        assert_eq!(missing_id.vector_store_id, None);
    }

    #[test]
    fn vertex_ai_debug_output_redacts_the_service_account_credentials() {
        let options: VertexAIVectorStoreOptions = serde_json::from_str(
            r#"{"vector_store_id": "vs_vertex", "vertex_credentials": "{\"private_key\": \"shh\"}"}"#,
        )
        .expect("vector_store_id and vertex_credentials match the struct");
        let debug_output = format!("{options:?}");
        assert!(!debug_output.contains("private_key"));
        assert!(!debug_output.contains("shh"));
    }

    #[test]
    fn s3_vectors_options_hold_a_bucket_and_their_aws_credentials() {
        let options: S3VectorsVectorStoreOptions = serde_json::from_str(
            r#"{
                "vector_bucket_name": "bucket",
                "index_name": "index",
                "dimension": 1536,
                "distance_metric": "cosine",
                "non_filterable_metadata_keys": ["source"],
                "aws_access_key_id": "AKIA",
                "aws_secret_access_key": "secret",
                "aws_session_token": "session",
                "aws_web_identity_token": "web-token",
                "aws_sts_endpoint": "https://sts.amazonaws.com",
                "aws_external_id": "external"
            }"#,
        )
        .expect("provider fields parse");
        assert_eq!(options.vector_bucket_name, Some("bucket".to_owned()));
        assert_eq!(
            options.distance_metric,
            Some(S3VectorsDistanceMetric::Cosine)
        );
        assert_eq!(
            options
                .aws_access_key_id
                .expect("aws_access_key_id was set")
                .expose_secret(),
            "AKIA"
        );
        assert_eq!(
            options
                .aws_secret_access_key
                .expect("aws_secret_access_key was set")
                .expose_secret(),
            "secret"
        );
    }

    #[test]
    fn s3_vectors_options_parse_with_only_a_vector_store_id() {
        let options: S3VectorsVectorStoreOptions =
            serde_json::from_str(r#"{"vector_store_id":"vs_1"}"#)
                .expect("store id is an alternative ingestion target");
        assert_eq!(options.vector_store_id, Some("vs_1".to_owned()));
        assert_eq!(options.vector_bucket_name, None);
    }

    #[test]
    fn s3_vectors_debug_output_redacts_every_secret_field() {
        let options: S3VectorsVectorStoreOptions = serde_json::from_str(
            r#"{
                "vector_bucket_name": "bucket",
                "aws_access_key_id": "akia-value",
                "aws_secret_access_key": "sak-value"
            }"#,
        )
        .expect("every field here matches the struct");
        let debug_output = format!("{options:?}");
        assert!(!debug_output.contains("akia-value"));
        assert!(!debug_output.contains("sak-value"));
    }

    #[test]
    fn vector_store_options_union_is_tagged_by_custom_llm_provider() {
        let openai: RAGIngestVectorStoreOptions =
            serde_json::from_str(r#"{"custom_llm_provider": "openai", "vector_store_id": "vs_1"}"#)
                .expect("openai is a known tag");
        assert!(matches!(openai, RAGIngestVectorStoreOptions::Openai(_)));

        let s3_vectors: RAGIngestVectorStoreOptions = serde_json::from_str(
            r#"{"custom_llm_provider": "s3_vectors", "vector_bucket_name": "bucket"}"#,
        )
        .expect("s3_vectors is a known tag");
        assert!(matches!(
            s3_vectors,
            RAGIngestVectorStoreOptions::S3Vectors(_)
        ));
    }

    #[test]
    fn ingest_options_bundles_every_section() {
        let options: RAGIngestOptions = serde_json::from_str(
            r#"{
                "name": "ingest-1",
                "ocr": {"model": "mistral-ocr"},
                "chunking_strategy": {"chunk_size": 500},
                "embedding": {"model": "text-embedding-3-small"},
                "vector_store": {"custom_llm_provider": "openai", "vector_store_id": "vs_1"}
            }"#,
        )
        .expect("every nested section matches its struct");
        assert_eq!(options.name, Some("ingest-1".to_owned()));
        assert!(options.ocr.is_some());
        assert!(options.chunking_strategy.is_some());
        assert!(options.embedding.is_some());
        assert!(options.vector_store.is_some());
    }

    #[test]
    fn ingest_response_carries_its_status() {
        let response: RAGIngestResponse = serde_json::from_str(
            r#"{"id": "ingest_1", "status": "in_progress", "vector_store_id": "vs_1"}"#,
        )
        .expect("status is a known variant");
        assert_eq!(response.id, "ingest_1");
        assert_eq!(response.status, RAGIngestStatus::InProgress);
        assert_eq!(response.vector_store_id, Some("vs_1".to_owned()));
        assert_eq!(response.file_id, None);
        assert_eq!(response.error, None);
    }

    #[rstest::rstest]
    #[case::openai(IngestionProvider::Openai, "openai", "OpenAIRAGIngestion")]
    #[case::bedrock(IngestionProvider::Bedrock, "bedrock", "BedrockRAGIngestion")]
    #[case::gemini(IngestionProvider::Gemini, "gemini", "GeminiRAGIngestion")]
    #[case::s3_vectors(IngestionProvider::S3Vectors, "s3_vectors", "S3VectorsRAGIngestion")]
    #[case::vertex_ai(IngestionProvider::VertexAi, "vertex_ai", "VertexAIRAGIngestion")]
    fn ingestion_registry_maps_every_provider(
        #[case] provider: IngestionProvider,
        #[case] wire_name: &str,
        #[case] class_name: &str,
    ) {
        assert_eq!(provider.wire_name_and_class(), (wire_name, class_name));
        assert_eq!(IngestionProvider::from_wire_name(wire_name), Some(provider));
    }

    #[test]
    fn ingestion_registry_has_exactly_five_providers() {
        assert_eq!(INGESTION_PROVIDERS.len(), 5);
        assert_eq!(IngestionProvider::from_wire_name("unknown"), None);
    }

    #[rstest::rstest]
    fn replays_the_rag_ingest_options_corpus(
        #[files("fuzz/corpus/rag_ingest_options/*")] path: PathBuf,
    ) {
        let bytes = fs::read(path).expect("corpus seed is readable");
        drop(serde_json::from_slice::<RAGIngestOptions>(&bytes));
    }
}
