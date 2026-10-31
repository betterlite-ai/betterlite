#![cfg(test)]

mod types {
    use secrecy::ExposeSecret as _;

    use crate::rag::document::{
        BedrockVectorStoreOptions, INGESTION_PROVIDERS, IngestionProvider,
        OpenAIVectorStoreOptions, ProviderIdentity as _, RAGChunkingStrategy,
        RAGIngestEmbeddingOptions, RAGIngestOCROptions, RAGIngestOptions, RAGIngestResponse,
        RAGIngestStatus, RAGIngestVectorStoreOptions, S3VectorsDistanceMetric,
        S3VectorsVectorStoreOptions, VertexAIVectorStoreOptions,
    };

    #[test]
    fn chunking_strategy_fields() {
        let strategy: RAGChunkingStrategy = serde_json::from_str(
            r#"{"chunk_size": 500, "chunk_overlap": 50, "separators": ["\n\n", "\n"]}"#,
        )
        .expect("every field here matches the struct");
        assert_eq!(strategy.chunk_size, 500);
        assert_eq!(strategy.chunk_overlap, 50);
        assert_eq!(
            strategy.separators,
            Some(vec!["\n\n".to_owned(), "\n".to_owned()])
        );
    }

    #[test]
    fn chunking_strategy_defaults_to_hardcoded_values() {
        let strategy: RAGChunkingStrategy =
            serde_json::from_str("{}").expect("every field has a default");
        assert_eq!(strategy.chunk_size, 1000);
        assert_eq!(strategy.chunk_overlap, 200);
        assert_eq!(strategy.separators, None);
        assert_eq!(strategy, RAGChunkingStrategy::default());
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
    fn vertex_ai_vector_store_options_requires_a_vector_store_id() {
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
        .expect("vector_store_id is required and present here");
        assert_eq!(options.vector_store_id, "vs_vertex");
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
        .unwrap_err();
        assert!(missing_id.to_string().contains("vector_store_id"));
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
    fn s3_vectors_options_requires_a_bucket_and_holds_its_aws_credentials() {
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
        .expect("vector_bucket_name is required and present here");
        assert_eq!(options.vector_bucket_name, "bucket");
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
        assert_eq!(provider.wire_name(), wire_name);
        assert_eq!(provider.ingestion_class_name(), class_name);
        assert_eq!(IngestionProvider::from_wire_name(wire_name), Some(provider));
    }

    #[test]
    fn ingestion_registry_has_exactly_five_providers() {
        assert_eq!(INGESTION_PROVIDERS.len(), 5);
        assert_eq!(IngestionProvider::from_wire_name("unknown"), None);
    }
}
