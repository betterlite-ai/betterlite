from dataclasses import fields
from typing import TypeAlias

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

from betterlite.types import rag as betterlite_rag
from betterlite.types import search as betterlite_search
from betterlite.types import utils as betterlite_utils
from betterlite.types import vector_store_files as betterlite_files
from betterlite.types import vector_stores as betterlite_stores

TypedDictKeys: TypeAlias = tuple[bool, tuple[str, ...], tuple[str, ...]]

OPENAI_OPTIONS: dict[str, JsonValue] = {"custom_llm_provider": "openai"}
BEDROCK_OPTIONS: dict[str, JsonValue] = {
    "custom_llm_provider": "bedrock",
    "s3_bucket": "bucket",
}
VERTEX_OPTIONS: dict[str, JsonValue] = {
    "custom_llm_provider": "vertex_ai",
    "vector_store_id": "corpus",
}
S3_VECTORS_OPTIONS: dict[str, JsonValue] = {
    "custom_llm_provider": "s3_vectors",
    "vector_bucket_name": "bucket",
}
UNKNOWN_OPTIONS: dict[str, JsonValue] = {"custom_llm_provider": "pinecone"}
RETRIEVAL: dict[str, JsonValue] = {
    "vector_store_id": "vs_1",
    "custom_llm_provider": "openai",
    "top_k": 3,
}
QUERY: dict[str, JsonValue] = {
    "model": "gpt-4o",
    "messages": [],
    "retrieval_config": RETRIEVAL,
    "extra_field": 1,
}
INGEST: dict[str, JsonValue] = {"ingest_options": {}, "extra_field": 1}

TYPED_DICT_KEYS: dict[str, TypedDictKeys] = {
    "LiteLLM_VectorStoreConfig": (False, (), ("litellm_params", "vector_store_name")),
    "LiteLLM_ManagedVectorStore": (
        False,
        (),
        (
            "created_at", "custom_llm_provider", "is_config", "litellm_credential_name",
            "litellm_params", "team_id", "updated_at", "user_id", "vector_store_description",
            "vector_store_id", "vector_store_metadata", "vector_store_name",
        ),
    ),
    "LiteLLM_ManagedVectorStoreListResponse": (
        False,
        (),
        ("current_page", "data", "object", "total_count", "total_pages"),
    ),
    "VectorStoreResultContent": (False, (), ("text", "type")),
    "VectorStoreSearchResult": (
        False,
        (),
        ("attributes", "content", "file_id", "filename", "score"),
    ),
    "VectorStoreSearchResponse": (False, (), ("data", "object", "search_query")),
    "VectorStoreSearchFailure": (
        True,
        ("custom_llm_provider", "error", "vector_store_id"),
        (),
    ),
    "VectorStoreSearchOptionalRequestParams": (
        False,
        (),
        ("filters", "max_num_results", "ranking_options", "rewrite_query"),
    ),
    "VectorStoreSearchRequest": (
        False,
        (),
        ("filters", "max_num_results", "query", "ranking_options", "rewrite_query"),
    ),
    "VertexSearchDataStoreExtraBody": (
        False,
        (),
        (
            "boostSpec", "canonicalFilter", "contentSearchSpec", "crowdingSpecs",
            "customRankingParams", "displaySpec", "facetSpecs", "filter", "imageQuery",
            "languageCode", "naturalLanguageQueryUnderstandingSpec", "offset",
            "oneBoxPageSize", "orderBy", "pageCategories", "pageSize", "pageToken", "params",
            "query", "queryExpansionSpec", "rankingExpression", "rankingExpressionBackend",
            "relevanceScoreSpec", "relevanceThreshold", "safeSearch", "searchAsYouTypeSpec",
            "spellCorrectionSpec", "userInfo", "userLabels", "userPseudoId",
        ),
    ),
    "VertexSearchEngineExtraBody": (
        False,
        (),
        (
            "boostSpec", "canonicalFilter", "contentSearchSpec", "crowdingSpecs",
            "customRankingParams", "dataStoreSpecs", "displaySpec", "facetSpecs", "filter",
            "imageQuery", "languageCode", "naturalLanguageQueryUnderstandingSpec",
            "numResultsPerDataStore", "offset", "oneBoxPageSize", "orderBy", "pageCategories",
            "pageSize", "pageToken", "params", "query", "queryExpansionSpec",
            "rankingExpression", "rankingExpressionBackend", "relevanceScoreSpec",
            "relevanceThreshold", "safeSearch", "searchAsYouTypeSpec", "spellCorrectionSpec",
            "userInfo", "userLabels", "userPseudoId",
        ),
    ),
    "VectorStoreExpirationPolicy": (False, (), ("anchor", "days")),
    "VectorStoreAutoChunkingStrategy": (False, (), ("type",)),
    "VectorStoreStaticChunkingStrategyConfig": (
        False,
        (),
        ("chunk_overlap_tokens", "max_chunk_size_tokens"),
    ),
    "VectorStoreStaticChunkingStrategy": (False, (), ("static", "type")),
    "VectorStoreChunkingStrategy": (False, (), ("static", "type")),
    "VectorStoreFileCounts": (
        False,
        (),
        ("cancelled", "completed", "failed", "in_progress", "total"),
    ),
    "VectorStoreCreateOptionalRequestParams": (
        False,
        (),
        ("chunking_strategy", "expires_after", "file_ids", "metadata", "name"),
    ),
    "VectorStoreCreateRequest": (
        False,
        (),
        ("chunking_strategy", "expires_after", "file_ids", "metadata", "name"),
    ),
    "VectorStoreCreateResponse": (
        False,
        (),
        (
            "bytes", "created_at", "expires_after", "expires_at", "file_counts", "id",
            "last_active_at", "metadata", "name", "object", "status",
        ),
    ),
    "BaseVectorStoreAuthCredentials": (False, (), ("headers", "query_params")),
    "VectorStoreIndexEndpoints": (True, ("read", "write"), ()),
    "VectorStoreFileStaticChunkingConfig": (
        False,
        (),
        ("chunk_overlap_tokens", "max_chunk_size_tokens"),
    ),
    "VectorStoreFileChunkingStrategy": (False, (), ("static", "type")),
    "VectorStoreFileObject": (
        False,
        (),
        (
            "attributes", "chunking_strategy", "created_at", "id", "last_error", "object",
            "status", "usage_bytes", "vector_store_id",
        ),
    ),
    "VectorStoreFileCreateRequest": (
        False,
        (),
        ("attributes", "chunking_strategy", "file_id"),
    ),
    "VectorStoreFileUpdateRequest": (False, (), ("attributes",)),
    "VectorStoreFileListQueryParams": (
        False,
        (),
        ("after", "before", "filter", "limit", "order"),
    ),
    "VectorStoreFileListResponse": (
        False,
        (),
        ("data", "first_id", "has_more", "last_id", "object"),
    ),
    "VectorStoreFileDeleteResponse": (False, (), ("deleted", "id", "object")),
    "VectorStoreFileContentTextPart": (False, (), ("text", "type")),
    "VectorStoreFileContentResponse": (
        False,
        (),
        ("attributes", "content", "file_id", "filename"),
    ),
    "VectorStoreFileAuthCredentials": (False, (), ("headers", "query_params")),
    "RAGChunkingStrategy": (False, (), ("chunk_overlap", "chunk_size", "separators")),
    "RAGIngestOCROptions": (False, (), ("model",)),
    "RAGIngestEmbeddingOptions": (False, (), ("model",)),
    "OpenAIVectorStoreOptions": (
        False,
        (),
        (
            "api_base", "api_key", "custom_llm_provider", "litellm_credential_name",
            "ttl_days", "vector_store_id",
        ),
    ),
    "BedrockVectorStoreOptions": (
        False,
        (),
        (
            "aws_access_key_id", "aws_external_id", "aws_profile_name", "aws_region_name",
            "aws_role_name", "aws_secret_access_key", "aws_session_name", "aws_session_token",
            "aws_sts_endpoint", "aws_web_identity_token", "custom_llm_provider",
            "data_source_id", "embedding_model", "ingestion_timeout",
            "litellm_credential_name", "s3_bucket", "s3_prefix", "vector_store_id",
            "wait_for_ingestion",
        ),
    ),
    "VertexAIVectorStoreOptions": (
        False,
        (),
        (
            "custom_llm_provider", "gcs_bucket", "import_timeout", "vector_store_id",
            "vertex_credentials", "vertex_location", "vertex_project", "wait_for_import",
        ),
    ),
    "S3VectorsVectorStoreOptions": (
        False,
        (),
        (
            "aws_access_key_id", "aws_external_id", "aws_profile_name", "aws_region_name",
            "aws_role_name", "aws_secret_access_key", "aws_session_name", "aws_session_token",
            "aws_sts_endpoint", "aws_web_identity_token", "custom_llm_provider", "dimension",
            "distance_metric", "index_name", "litellm_credential_name",
            "non_filterable_metadata_keys", "vector_bucket_name",
        ),
    ),
    "RAGIngestOptions": (
        False,
        (),
        ("chunking_strategy", "embedding", "name", "ocr", "vector_store"),
    ),
    "RAGIngestResponse": (False, (), ("error", "file_id", "id", "status", "vector_store_id")),
    "RAGRetrievalConfig": (
        False,
        (),
        ("custom_llm_provider", "filters", "retrieval_filter", "top_k", "vector_store_id"),
    ),
    "RAGRerankConfig": (False, (), ("enabled", "model", "return_documents", "top_n")),
    "SearchToolLiteLLMParams": (
        False,
        ("search_provider",),
        ("api_base", "api_key", "max_retries", "timeout"),
    ),
    "SearchTool": (
        False,
        ("litellm_params", "search_tool_name"),
        ("created_at", "search_tool_id", "search_tool_info", "updated_at"),
    ),
    "SearchToolInfoResponse": (
        False,
        (),
        (
            "created_at", "is_from_config", "litellm_params", "search_tool_id",
            "search_tool_info", "search_tool_name", "updated_at",
        ),
    ),
    "ListSearchToolsResponse": (True, ("search_tools",), ()),
    "AvailableSearchProvider": (True, ("provider_name", "ui_friendly_name"), ()),
}

OUR_TYPED_DICTS: dict[str, tuple[bool, frozenset[str], frozenset[str]]] = {
    "LiteLLM_VectorStoreConfig": (
        betterlite_stores.LiteLLM_VectorStoreConfig.__total__,
        betterlite_stores.LiteLLM_VectorStoreConfig.__required_keys__,
        betterlite_stores.LiteLLM_VectorStoreConfig.__optional_keys__,
    ),
    "LiteLLM_ManagedVectorStore": (
        betterlite_stores.LiteLLM_ManagedVectorStore.__total__,
        betterlite_stores.LiteLLM_ManagedVectorStore.__required_keys__,
        betterlite_stores.LiteLLM_ManagedVectorStore.__optional_keys__,
    ),
    "LiteLLM_ManagedVectorStoreListResponse": (
        betterlite_stores.LiteLLM_ManagedVectorStoreListResponse.__total__,
        betterlite_stores.LiteLLM_ManagedVectorStoreListResponse.__required_keys__,
        betterlite_stores.LiteLLM_ManagedVectorStoreListResponse.__optional_keys__,
    ),
    "VectorStoreResultContent": (
        betterlite_stores.VectorStoreResultContent.__total__,
        betterlite_stores.VectorStoreResultContent.__required_keys__,
        betterlite_stores.VectorStoreResultContent.__optional_keys__,
    ),
    "VectorStoreSearchResult": (
        betterlite_stores.VectorStoreSearchResult.__total__,
        betterlite_stores.VectorStoreSearchResult.__required_keys__,
        betterlite_stores.VectorStoreSearchResult.__optional_keys__,
    ),
    "VectorStoreSearchResponse": (
        betterlite_stores.VectorStoreSearchResponse.__total__,
        betterlite_stores.VectorStoreSearchResponse.__required_keys__,
        betterlite_stores.VectorStoreSearchResponse.__optional_keys__,
    ),
    "VectorStoreSearchFailure": (
        betterlite_stores.VectorStoreSearchFailure.__total__,
        betterlite_stores.VectorStoreSearchFailure.__required_keys__,
        betterlite_stores.VectorStoreSearchFailure.__optional_keys__,
    ),
    "VectorStoreSearchOptionalRequestParams": (
        betterlite_stores.VectorStoreSearchOptionalRequestParams.__total__,
        betterlite_stores.VectorStoreSearchOptionalRequestParams.__required_keys__,
        betterlite_stores.VectorStoreSearchOptionalRequestParams.__optional_keys__,
    ),
    "VectorStoreSearchRequest": (
        betterlite_stores.VectorStoreSearchRequest.__total__,
        betterlite_stores.VectorStoreSearchRequest.__required_keys__,
        betterlite_stores.VectorStoreSearchRequest.__optional_keys__,
    ),
    "VertexSearchDataStoreExtraBody": (
        betterlite_stores.VertexSearchDataStoreExtraBody.__total__,
        betterlite_stores.VertexSearchDataStoreExtraBody.__required_keys__,
        betterlite_stores.VertexSearchDataStoreExtraBody.__optional_keys__,
    ),
    "VertexSearchEngineExtraBody": (
        betterlite_stores.VertexSearchEngineExtraBody.__total__,
        betterlite_stores.VertexSearchEngineExtraBody.__required_keys__,
        betterlite_stores.VertexSearchEngineExtraBody.__optional_keys__,
    ),
    "VectorStoreExpirationPolicy": (
        betterlite_stores.VectorStoreExpirationPolicy.__total__,
        betterlite_stores.VectorStoreExpirationPolicy.__required_keys__,
        betterlite_stores.VectorStoreExpirationPolicy.__optional_keys__,
    ),
    "VectorStoreAutoChunkingStrategy": (
        betterlite_stores.VectorStoreAutoChunkingStrategy.__total__,
        betterlite_stores.VectorStoreAutoChunkingStrategy.__required_keys__,
        betterlite_stores.VectorStoreAutoChunkingStrategy.__optional_keys__,
    ),
    "VectorStoreStaticChunkingStrategyConfig": (
        betterlite_stores.VectorStoreStaticChunkingStrategyConfig.__total__,
        betterlite_stores.VectorStoreStaticChunkingStrategyConfig.__required_keys__,
        betterlite_stores.VectorStoreStaticChunkingStrategyConfig.__optional_keys__,
    ),
    "VectorStoreStaticChunkingStrategy": (
        betterlite_stores.VectorStoreStaticChunkingStrategy.__total__,
        betterlite_stores.VectorStoreStaticChunkingStrategy.__required_keys__,
        betterlite_stores.VectorStoreStaticChunkingStrategy.__optional_keys__,
    ),
    "VectorStoreChunkingStrategy": (
        betterlite_stores.VectorStoreChunkingStrategy.__total__,
        betterlite_stores.VectorStoreChunkingStrategy.__required_keys__,
        betterlite_stores.VectorStoreChunkingStrategy.__optional_keys__,
    ),
    "VectorStoreFileCounts": (
        betterlite_stores.VectorStoreFileCounts.__total__,
        betterlite_stores.VectorStoreFileCounts.__required_keys__,
        betterlite_stores.VectorStoreFileCounts.__optional_keys__,
    ),
    "VectorStoreCreateOptionalRequestParams": (
        betterlite_stores.VectorStoreCreateOptionalRequestParams.__total__,
        betterlite_stores.VectorStoreCreateOptionalRequestParams.__required_keys__,
        betterlite_stores.VectorStoreCreateOptionalRequestParams.__optional_keys__,
    ),
    "VectorStoreCreateRequest": (
        betterlite_stores.VectorStoreCreateRequest.__total__,
        betterlite_stores.VectorStoreCreateRequest.__required_keys__,
        betterlite_stores.VectorStoreCreateRequest.__optional_keys__,
    ),
    "VectorStoreCreateResponse": (
        betterlite_stores.VectorStoreCreateResponse.__total__,
        betterlite_stores.VectorStoreCreateResponse.__required_keys__,
        betterlite_stores.VectorStoreCreateResponse.__optional_keys__,
    ),
    "BaseVectorStoreAuthCredentials": (
        betterlite_stores.BaseVectorStoreAuthCredentials.__total__,
        betterlite_stores.BaseVectorStoreAuthCredentials.__required_keys__,
        betterlite_stores.BaseVectorStoreAuthCredentials.__optional_keys__,
    ),
    "VectorStoreIndexEndpoints": (
        betterlite_stores.VectorStoreIndexEndpoints.__total__,
        betterlite_stores.VectorStoreIndexEndpoints.__required_keys__,
        betterlite_stores.VectorStoreIndexEndpoints.__optional_keys__,
    ),
    "VectorStoreFileStaticChunkingConfig": (
        betterlite_files.VectorStoreFileStaticChunkingConfig.__total__,
        betterlite_files.VectorStoreFileStaticChunkingConfig.__required_keys__,
        betterlite_files.VectorStoreFileStaticChunkingConfig.__optional_keys__,
    ),
    "VectorStoreFileChunkingStrategy": (
        betterlite_files.VectorStoreFileChunkingStrategy.__total__,
        betterlite_files.VectorStoreFileChunkingStrategy.__required_keys__,
        betterlite_files.VectorStoreFileChunkingStrategy.__optional_keys__,
    ),
    "VectorStoreFileObject": (
        betterlite_files.VectorStoreFileObject.__total__,
        betterlite_files.VectorStoreFileObject.__required_keys__,
        betterlite_files.VectorStoreFileObject.__optional_keys__,
    ),
    "VectorStoreFileCreateRequest": (
        betterlite_files.VectorStoreFileCreateRequest.__total__,
        betterlite_files.VectorStoreFileCreateRequest.__required_keys__,
        betterlite_files.VectorStoreFileCreateRequest.__optional_keys__,
    ),
    "VectorStoreFileUpdateRequest": (
        betterlite_files.VectorStoreFileUpdateRequest.__total__,
        betterlite_files.VectorStoreFileUpdateRequest.__required_keys__,
        betterlite_files.VectorStoreFileUpdateRequest.__optional_keys__,
    ),
    "VectorStoreFileListQueryParams": (
        betterlite_files.VectorStoreFileListQueryParams.__total__,
        betterlite_files.VectorStoreFileListQueryParams.__required_keys__,
        betterlite_files.VectorStoreFileListQueryParams.__optional_keys__,
    ),
    "VectorStoreFileListResponse": (
        betterlite_files.VectorStoreFileListResponse.__total__,
        betterlite_files.VectorStoreFileListResponse.__required_keys__,
        betterlite_files.VectorStoreFileListResponse.__optional_keys__,
    ),
    "VectorStoreFileDeleteResponse": (
        betterlite_files.VectorStoreFileDeleteResponse.__total__,
        betterlite_files.VectorStoreFileDeleteResponse.__required_keys__,
        betterlite_files.VectorStoreFileDeleteResponse.__optional_keys__,
    ),
    "VectorStoreFileContentTextPart": (
        betterlite_files.VectorStoreFileContentTextPart.__total__,
        betterlite_files.VectorStoreFileContentTextPart.__required_keys__,
        betterlite_files.VectorStoreFileContentTextPart.__optional_keys__,
    ),
    "VectorStoreFileContentResponse": (
        betterlite_files.VectorStoreFileContentResponse.__total__,
        betterlite_files.VectorStoreFileContentResponse.__required_keys__,
        betterlite_files.VectorStoreFileContentResponse.__optional_keys__,
    ),
    "VectorStoreFileAuthCredentials": (
        betterlite_files.VectorStoreFileAuthCredentials.__total__,
        betterlite_files.VectorStoreFileAuthCredentials.__required_keys__,
        betterlite_files.VectorStoreFileAuthCredentials.__optional_keys__,
    ),
    "RAGChunkingStrategy": (
        betterlite_rag.RAGChunkingStrategy.__total__,
        betterlite_rag.RAGChunkingStrategy.__required_keys__,
        betterlite_rag.RAGChunkingStrategy.__optional_keys__,
    ),
    "RAGIngestOCROptions": (
        betterlite_rag.RAGIngestOCROptions.__total__,
        betterlite_rag.RAGIngestOCROptions.__required_keys__,
        betterlite_rag.RAGIngestOCROptions.__optional_keys__,
    ),
    "RAGIngestEmbeddingOptions": (
        betterlite_rag.RAGIngestEmbeddingOptions.__total__,
        betterlite_rag.RAGIngestEmbeddingOptions.__required_keys__,
        betterlite_rag.RAGIngestEmbeddingOptions.__optional_keys__,
    ),
    "OpenAIVectorStoreOptions": (
        betterlite_rag.OpenAIVectorStoreOptions.__total__,
        betterlite_rag.OpenAIVectorStoreOptions.__required_keys__,
        betterlite_rag.OpenAIVectorStoreOptions.__optional_keys__,
    ),
    "BedrockVectorStoreOptions": (
        betterlite_rag.BedrockVectorStoreOptions.__total__,
        betterlite_rag.BedrockVectorStoreOptions.__required_keys__,
        betterlite_rag.BedrockVectorStoreOptions.__optional_keys__,
    ),
    "VertexAIVectorStoreOptions": (
        betterlite_rag.VertexAIVectorStoreOptions.__total__,
        betterlite_rag.VertexAIVectorStoreOptions.__required_keys__,
        betterlite_rag.VertexAIVectorStoreOptions.__optional_keys__,
    ),
    "S3VectorsVectorStoreOptions": (
        betterlite_rag.S3VectorsVectorStoreOptions.__total__,
        betterlite_rag.S3VectorsVectorStoreOptions.__required_keys__,
        betterlite_rag.S3VectorsVectorStoreOptions.__optional_keys__,
    ),
    "RAGIngestOptions": (
        betterlite_rag.RAGIngestOptions.__total__,
        betterlite_rag.RAGIngestOptions.__required_keys__,
        betterlite_rag.RAGIngestOptions.__optional_keys__,
    ),
    "RAGIngestResponse": (
        betterlite_rag.RAGIngestResponse.__total__,
        betterlite_rag.RAGIngestResponse.__required_keys__,
        betterlite_rag.RAGIngestResponse.__optional_keys__,
    ),
    "RAGRetrievalConfig": (
        betterlite_rag.RAGRetrievalConfig.__total__,
        betterlite_rag.RAGRetrievalConfig.__required_keys__,
        betterlite_rag.RAGRetrievalConfig.__optional_keys__,
    ),
    "RAGRerankConfig": (
        betterlite_rag.RAGRerankConfig.__total__,
        betterlite_rag.RAGRerankConfig.__required_keys__,
        betterlite_rag.RAGRerankConfig.__optional_keys__,
    ),
    "SearchToolLiteLLMParams": (
        betterlite_search.SearchToolLiteLLMParams.__total__,
        betterlite_search.SearchToolLiteLLMParams.__required_keys__,
        betterlite_search.SearchToolLiteLLMParams.__optional_keys__,
    ),
    "SearchTool": (
        betterlite_search.SearchTool.__total__,
        betterlite_search.SearchTool.__required_keys__,
        betterlite_search.SearchTool.__optional_keys__,
    ),
    "SearchToolInfoResponse": (
        betterlite_search.SearchToolInfoResponse.__total__,
        betterlite_search.SearchToolInfoResponse.__required_keys__,
        betterlite_search.SearchToolInfoResponse.__optional_keys__,
    ),
    "ListSearchToolsResponse": (
        betterlite_search.ListSearchToolsResponse.__total__,
        betterlite_search.ListSearchToolsResponse.__required_keys__,
        betterlite_search.ListSearchToolsResponse.__optional_keys__,
    ),
    "AvailableSearchProvider": (
        betterlite_search.AvailableSearchProvider.__total__,
        betterlite_search.AvailableSearchProvider.__required_keys__,
        betterlite_search.AvailableSearchProvider.__optional_keys__,
    ),
}

ENUM_MEMBERS: dict[str, dict[str, str]] = {
    "SupportedVectorStoreIntegrations": {"BEDROCK": "bedrock", "RAGFLOW": "ragflow"},
    "VectorStoreIndexType": {"READ": "read", "WRITE": "write"},
    "VectorStoreFileStatus": {
        "IN_PROGRESS": "in_progress",
        "COMPLETED": "completed",
        "FAILED": "failed",
        "CANCELLED": "cancelled",
    },
    "VectorStoreFileChunkingStrategyType": {"AUTO": "auto", "STATIC": "static"},
}

MODELS: dict[str, type[BaseModel]] = {
    "VectorStoreUpdateRequest": betterlite_stores.VectorStoreUpdateRequest,
    "VectorStoreDeleteRequest": betterlite_stores.VectorStoreDeleteRequest,
    "VectorStoreInfoRequest": betterlite_stores.VectorStoreInfoRequest,
    "IndexCreateLiteLLMParams": betterlite_stores.IndexCreateLiteLLMParams,
    "IndexCreateRequest": betterlite_stores.IndexCreateRequest,
    "LiteLLM_ManagedVectorStoreIndex": betterlite_stores.LiteLLM_ManagedVectorStoreIndex,
    "IndexListResponse": betterlite_stores.IndexListResponse,
    "RAGIngestRequest": betterlite_rag.RAGIngestRequest,
    "RAGQueryRequest": betterlite_rag.RAGQueryRequest,
}

MODEL_FIELDS: dict[str, tuple[str, ...]] = {
    "VectorStoreUpdateRequest": (
        "vector_store_id",
        "custom_llm_provider",
        "vector_store_name",
        "vector_store_description",
        "vector_store_metadata",
    ),
    "VectorStoreDeleteRequest": ("vector_store_id",),
    "VectorStoreInfoRequest": ("vector_store_id",),
    "IndexCreateLiteLLMParams": ("vector_store_index", "vector_store_name"),
    "IndexCreateRequest": ("index_name", "litellm_params", "index_info"),
    "LiteLLM_ManagedVectorStoreIndex": (
        "id",
        "index_name",
        "litellm_params",
        "index_info",
        "created_at",
        "created_by",
        "updated_at",
        "updated_by",
    ),
    "IndexListResponse": ("object", "data"),
    "RAGIngestRequest": ("file_url", "file_id", "ingest_options"),
    "RAGQueryRequest": ("model", "messages", "retrieval_config", "rerank", "stream"),
}

MODEL_REQUIRED: dict[str, tuple[str, ...]] = {
    "VectorStoreUpdateRequest": ("vector_store_id",),
    "VectorStoreDeleteRequest": ("vector_store_id",),
    "VectorStoreInfoRequest": ("vector_store_id",),
    "IndexCreateLiteLLMParams": ("vector_store_index", "vector_store_name"),
    "IndexCreateRequest": ("index_name", "litellm_params"),
    "LiteLLM_ManagedVectorStoreIndex": ("id", "index_name", "litellm_params"),
    "IndexListResponse": ("data",),
    "RAGIngestRequest": ("ingest_options",),
    "RAGQueryRequest": ("model", "messages", "retrieval_config"),
}

MODEL_DEFAULTS: dict[str, dict[str, JsonValue]] = {
    "VectorStoreUpdateRequest": {
        "custom_llm_provider": None,
        "vector_store_name": None,
        "vector_store_description": None,
        "vector_store_metadata": None,
    },
    "VectorStoreDeleteRequest": {},
    "VectorStoreInfoRequest": {},
    "IndexCreateLiteLLMParams": {},
    "IndexCreateRequest": {"index_info": None},
    "LiteLLM_ManagedVectorStoreIndex": {
        "index_info": None,
        "created_at": None,
        "created_by": None,
        "updated_at": None,
        "updated_by": None,
    },
    "IndexListResponse": {"object": "list"},
    "RAGIngestRequest": {"file_url": None, "file_id": None},
    "RAGQueryRequest": {"rerank": None, "stream": False},
}

MODEL_EXTRA: dict[str, JsonValue] = {
    "VectorStoreUpdateRequest": None,
    "VectorStoreDeleteRequest": None,
    "VectorStoreInfoRequest": None,
    "IndexCreateLiteLLMParams": None,
    "IndexCreateRequest": None,
    "LiteLLM_ManagedVectorStoreIndex": None,
    "IndexListResponse": None,
    "RAGIngestRequest": "allow",
    "RAGQueryRequest": "allow",
}


@pytest.mark.parametrize("name", sorted(TYPED_DICT_KEYS))
def test_names_typed_dicts(name: str) -> None:
    total, required, optional = TYPED_DICT_KEYS[name]
    ours_total, ours_required, ours_optional = OUR_TYPED_DICTS[name]

    assert ours_total == total
    assert ours_required == frozenset(required)
    assert ours_optional == frozenset(optional)


@pytest.mark.parametrize("name", sorted(MODELS))
def test_names_models(name: str) -> None:
    model = MODELS[name]
    model_fields = model.model_fields

    assert tuple(model_fields) == MODEL_FIELDS[name]
    required = tuple(field for field, info in model_fields.items() if info.is_required())
    assert required == MODEL_REQUIRED[name]
    defaults = {
        field: info.default for field, info in model_fields.items() if not info.is_required()
    }
    assert defaults == MODEL_DEFAULTS[name]
    assert model.model_config.get("extra") == MODEL_EXTRA[name]


def test_names_enums() -> None:
    integrations = {
        member.name: member.value for member in betterlite_stores.SupportedVectorStoreIntegrations
    }
    assert integrations == ENUM_MEMBERS["SupportedVectorStoreIntegrations"]

    index_type = {member.name: member.value for member in betterlite_stores.VectorStoreIndexType}
    assert index_type == ENUM_MEMBERS["VectorStoreIndexType"]

    status = {member.name: member.value for member in betterlite_files.VectorStoreFileStatus}
    assert status == ENUM_MEMBERS["VectorStoreFileStatus"]

    chunking = {
        member.name: member.value
        for member in betterlite_files.VectorStoreFileChunkingStrategyType
    }
    assert chunking == ENUM_MEMBERS["VectorStoreFileChunkingStrategyType"]


def test_names_literal_aliases() -> None:
    failure_mode = TypeAdapter(betterlite_stores.VectorStoreSearchFailureMode)
    assert failure_mode.validate_python("annotate") == "annotate"
    assert failure_mode.validate_python("error") == "error"
    with pytest.raises(ValidationError):
        failure_mode.validate_python("ignore")

    params = TypeAdapter(betterlite_stores.VECTOR_STORE_OPENAI_PARAMS)
    for value in ("filters", "max_num_results", "ranking_options", "rewrite_query"):
        assert params.validate_python(value) == value
    with pytest.raises(ValidationError):
        params.validate_python("temperature")


def test_names_rag_option_literals() -> None:
    openai = TypeAdapter(betterlite_rag.OpenAIVectorStoreOptions)
    assert openai.validate_python(OPENAI_OPTIONS) == OPENAI_OPTIONS
    with pytest.raises(ValidationError):
        openai.validate_python(BEDROCK_OPTIONS)

    bedrock = TypeAdapter(betterlite_rag.BedrockVectorStoreOptions)
    assert bedrock.validate_python(BEDROCK_OPTIONS) == BEDROCK_OPTIONS
    with pytest.raises(ValidationError):
        bedrock.validate_python(OPENAI_OPTIONS)

    vertex = TypeAdapter(betterlite_rag.VertexAIVectorStoreOptions)
    assert vertex.validate_python(VERTEX_OPTIONS) == VERTEX_OPTIONS
    with pytest.raises(ValidationError):
        vertex.validate_python(S3_VECTORS_OPTIONS)

    vectors = TypeAdapter(betterlite_rag.S3VectorsVectorStoreOptions)
    assert vectors.validate_python(S3_VECTORS_OPTIONS) == S3_VECTORS_OPTIONS
    with pytest.raises(ValidationError):
        vectors.validate_python(VERTEX_OPTIONS)

    base_vector: dict[str, JsonValue] = {
        "custom_llm_provider": "s3_vectors",
        "vector_bucket_name": "bucket",
    }
    for distance_metric in ("cosine", "euclidean"):
        value = {**base_vector, "distance_metric": distance_metric}
        assert vectors.validate_python(value) == value
    with pytest.raises(ValidationError):
        vectors.validate_python({**base_vector, "distance_metric": "manhattan"})


def test_names_rag_options_union() -> None:
    options = TypeAdapter(betterlite_rag.RAGIngestVectorStoreOptions)
    for data in (OPENAI_OPTIONS, BEDROCK_OPTIONS, VERTEX_OPTIONS, S3_VECTORS_OPTIONS):
        assert options.validate_python(data) == data
    with pytest.raises(ValidationError):
        options.validate_python(UNKNOWN_OPTIONS)


def test_names_store_literals() -> None:
    managed_list = TypeAdapter(betterlite_stores.LiteLLM_ManagedVectorStoreListResponse)
    assert managed_list.validate_python({"object": "list"}) == {"object": "list"}
    with pytest.raises(ValidationError):
        managed_list.validate_python({"object": "page"})

    search_response = TypeAdapter(betterlite_stores.VectorStoreSearchResponse)
    search_object: dict[str, JsonValue] = {"object": "vector_store.search_results.page"}
    assert search_response.validate_python(search_object) == search_object
    with pytest.raises(ValidationError):
        search_response.validate_python({"object": "vector_store.search_results"})

    expiration = TypeAdapter(betterlite_stores.VectorStoreExpirationPolicy)
    expiration_value: dict[str, JsonValue] = {"anchor": "last_active_at", "days": 7}
    assert expiration.validate_python(expiration_value) == expiration_value
    with pytest.raises(ValidationError):
        expiration.validate_python({"anchor": "created_at"})

    auto = TypeAdapter(betterlite_stores.VectorStoreAutoChunkingStrategy)
    assert auto.validate_python({"type": "auto"}) == {"type": "auto"}
    with pytest.raises(ValidationError):
        auto.validate_python({"type": "static"})

    static = TypeAdapter(betterlite_stores.VectorStoreStaticChunkingStrategy)
    static_value: dict[str, JsonValue] = {
        "type": "static",
        "static": {"max_chunk_size_tokens": 800, "chunk_overlap_tokens": 400},
    }
    assert static.validate_python(static_value) == static_value
    with pytest.raises(ValidationError):
        static.validate_python({"type": "auto"})

    chunking = TypeAdapter(betterlite_stores.VectorStoreChunkingStrategy)
    for strategy_type in ("auto", "static"):
        assert chunking.validate_python({"type": strategy_type}) == {"type": strategy_type}
    with pytest.raises(ValidationError):
        chunking.validate_python({"type": "none"})

    created = TypeAdapter(betterlite_stores.VectorStoreCreateResponse)
    for status in ("expired", "in_progress", "completed"):
        value = {"object": "vector_store", "status": status}
        assert created.validate_python(value) == value
    with pytest.raises(ValidationError):
        created.validate_python({"status": "done"})

    endpoints = TypeAdapter(betterlite_stores.VectorStoreIndexEndpoints)
    methods = ("GET", "POST", "PUT", "DELETE", "PATCH")
    assert endpoints.validate_python(
        {"read": [[method, "/v1/x"] for method in methods], "write": []},
    ) == {"read": [(method, "/v1/x") for method in methods], "write": []}
    with pytest.raises(ValidationError):
        endpoints.validate_python({"read": [["FETCH", "/v1/x"]], "write": []})


def test_names_file_literals() -> None:
    file_object = TypeAdapter(betterlite_files.VectorStoreFileObject)
    object_value: dict[str, JsonValue] = {"object": "vector_store.file", "status": "completed"}
    assert file_object.validate_python(object_value) == object_value
    with pytest.raises(ValidationError):
        file_object.validate_python({"status": "done"})

    file_strategy = TypeAdapter(betterlite_files.VectorStoreFileChunkingStrategy)
    for strategy_type in ("auto", "static"):
        assert file_strategy.validate_python({"type": strategy_type}) == {
            "type": strategy_type,
        }
    with pytest.raises(ValidationError):
        file_strategy.validate_python({"type": "none"})

    query_params = TypeAdapter(betterlite_files.VectorStoreFileListQueryParams)
    for file_status in ("in_progress", "completed", "failed", "cancelled"):
        value = {"filter": file_status}
        assert query_params.validate_python(value) == value
    for order in ("asc", "desc"):
        value = {"order": order}
        assert query_params.validate_python(value) == value
    with pytest.raises(ValidationError):
        query_params.validate_python({"order": "up"})

    file_list = TypeAdapter(betterlite_files.VectorStoreFileListResponse)
    assert file_list.validate_python({"object": "list"}) == {"object": "list"}
    with pytest.raises(ValidationError):
        file_list.validate_python({"object": "page"})

    deleted = TypeAdapter(betterlite_files.VectorStoreFileDeleteResponse)
    deleted_value: dict[str, JsonValue] = {"object": "vector_store.file.deleted"}
    assert deleted.validate_python(deleted_value) == deleted_value
    with pytest.raises(ValidationError):
        deleted.validate_python({"object": "vector_store.file"})

    text_part = TypeAdapter(betterlite_files.VectorStoreFileContentTextPart)
    text_value: dict[str, JsonValue] = {"type": "text", "text": "body"}
    assert text_part.validate_python(text_value) == text_value
    with pytest.raises(ValidationError):
        text_part.validate_python({"type": "image"})


def test_names_rag_literals() -> None:
    ingest_response = TypeAdapter(betterlite_rag.RAGIngestResponse)
    for status in ("completed", "in_progress", "failed"):
        assert ingest_response.validate_python({"status": status}) == {"status": status}
    with pytest.raises(ValidationError):
        ingest_response.validate_python({"status": "done"})


def test_names_model_defaults() -> None:
    index = betterlite_stores.IndexListResponse.model_validate({"data": []})
    assert index.object == "list"
    assert index.data == ()

    query = betterlite_rag.RAGQueryRequest.model_validate(QUERY)
    assert query.stream is False
    assert query.rerank is None
    assert query.model_extra == {"extra_field": 1}

    ingest = betterlite_rag.RAGIngestRequest.model_validate(INGEST)
    assert ingest.ingest_options == {}
    assert ingest.model_extra == {"extra_field": 1}

    with pytest.raises(ValidationError):
        betterlite_stores.VectorStoreDeleteRequest.model_validate({})


def test_names_tool_params() -> None:
    params = betterlite_stores.VectorStoreToolParams()

    assert [field.name for field in fields(params)] == [
        "filters",
        "max_num_results",
        "ranking_options",
    ]
    assert params.filters is None
    assert params.max_num_results is None
    assert params.ranking_options is None
    assert params.to_dict() == {}

    filled = betterlite_stores.VectorStoreToolParams(
        filters={"k": [1, 2]},
        max_num_results=3,
        ranking_options={"score": 0.5},
    )
    assert filled.to_dict() == {
        "filters": {"k": [1, 2]},
        "max_num_results": 3,
        "ranking_options": {"score": 0.5},
    }
    assert betterlite_stores.VectorStoreToolParams(filters={}).to_dict() == {"filters": {}}


def test_names_rag_query_response() -> None:
    response = betterlite_rag.RAGQueryResponse

    assert response.__mro__[1] is betterlite_utils.ModelResponse
    assert "choices" in response.model_fields
