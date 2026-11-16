#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "foundry_agent_requests" => agent_requests,
    "foundry_agent_results" => agent_results,
    "foundry_agent_runs" => agent_runs,
    "foundry_anthropic" => anthropic,
    "foundry_anthropic_count" => anthropic_count,
    "foundry_anthropic_messages" => anthropic_messages,
    "foundry_chat" => chat,
    "foundry_credentials" => credentials,
    "foundry_embeddings" => embeddings,
    "foundry_image_edits" => image_edits,
    "foundry_image_generation" => image_generation,
    "foundry_mai_images" => mai_images,
    "foundry_model_router" => model_router,
    "foundry_passthrough" => passthrough,
    "foundry_rejected_fields" => rejected_fields,
    "foundry_rerank" => rerank,
    "foundry_responses" => responses,
    "foundry_router_cost" => router_cost,
    "foundry_vector_stores" => vector_stores,
}
