#![cfg(test)]

use betterlite_models::catalog::{ModelCatalog, UnknownFields};

mod accounting;
mod encoding;
mod model;

const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "gpt-4o-mini": {"litellm_provider": "openai"},
    "gpt-4o-2024-05-13": {"litellm_provider": "openai"},
    "chatgpt-4o-latest": {"litellm_provider": "openai"},
    "my-gpt-4o-clone": {"litellm_provider": "openai"},
    "gpt-4": {"litellm_provider": "openai"},
    "gpt-4-0613": {"litellm_provider": "openai"},
    "gpt-4.1": {"litellm_provider": "openai"},
    "gpt-5": {"litellm_provider": "openai"},
    "o1": {"litellm_provider": "openai"},
    "gpt-oss-20b": {"litellm_provider": "openai"},
    "gpt-3.5-turbo": {"litellm_provider": "openai"},
    "gpt-3.5-turbo-0301": {"litellm_provider": "openai"},
    "text-embedding-3-small": {"litellm_provider": "openai"},
    "canonical-model": {"litellm_provider": "openai", "aliases": ["openai-alias"]},
    "azure-canonical": {"litellm_provider": "azure", "aliases": ["azure-alias"]},
    "ft:gpt-4o-2024-08-06": {"litellm_provider": "openai"},
    "ft:gpt-4o:org": {"litellm_provider": "openai"},
    "azure/gpt-4o": {"litellm_provider": "azure"}
}"#;

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap()
}

fn empty_catalog() -> ModelCatalog {
    ModelCatalog::parse(b"{}", UnknownFields::Reject).unwrap()
}
