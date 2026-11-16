use betterlite_context::resource_config_selection::responses::{
    AzureResponsesFacts, MantleResponses, ResponsesModelFacts, select_responses_config,
};
use betterlite_models::identity::provider_id::ProviderId;

use crate::data_cloud_dispatch::select::{EmbeddingConfig, embedding_config};

#[test]
fn embedding_config_selects_the_three_providers() {
    assert_eq!(
        embedding_config(ProviderId::Databricks),
        Some(EmbeddingConfig::Databricks)
    );
    assert_eq!(
        embedding_config(ProviderId::Snowflake),
        Some(EmbeddingConfig::Snowflake)
    );
    assert_eq!(
        embedding_config(ProviderId::Oci),
        Some(EmbeddingConfig::Oci)
    );
    assert_eq!(embedding_config(ProviderId::Openai), None);
}

#[test]
fn responses_selection_keeps_the_databricks_gpt_branch() {
    let facts = ResponsesModelFacts {
        azure: AzureResponsesFacts::default(),
        github_copilot_responses: false,
        mantle: MantleResponses::default(),
    };
    assert_eq!(
        select_responses_config(Some(ProviderId::Databricks), Some("databricks-gpt-5"), &facts),
        Some(betterlite_context::resource_config_selection::responses::ResponsesConfigTag::Databricks)
    );
    assert_eq!(
        select_responses_config(
            Some(ProviderId::Databricks),
            Some("databricks-llama-3"),
            &facts
        ),
        None
    );
    assert_eq!(
        select_responses_config(Some(ProviderId::Databricks), None, &facts),
        None
    );
}
