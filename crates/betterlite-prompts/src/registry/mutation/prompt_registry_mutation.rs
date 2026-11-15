pub mod callback_build;
pub mod callbacks;
pub mod error;
pub mod initializers;
pub mod methods;

#[path = "prompt_registry_mutation/tests/build_prompt_callback.rs"]
mod build_prompt_callback;
#[path = "prompt_registry_mutation/tests/delete_by_base_id.rs"]
mod delete_by_base_id;
#[path = "prompt_registry_mutation/tests/initialize_idempotent.rs"]
mod initialize_idempotent;
#[path = "prompt_registry_mutation/tests/initialize_prompt.rs"]
mod initialize_prompt;
#[path = "prompt_registry_mutation/tests/initializer_discovery.rs"]
mod initializer_discovery;
#[path = "prompt_registry_mutation/tests/registry_population.rs"]
mod registry_population;
#[path = "prompt_registry_mutation/tests/reload_order.rs"]
mod reload_order;
#[path = "prompt_registry_mutation/tests/reload_safety.rs"]
mod reload_safety;
#[path = "prompt_registry_mutation/tests/remove_prompt.rs"]
mod remove_prompt;
#[path = "prompt_registry_mutation/tests/support.rs"]
mod support;
#[path = "prompt_registry_mutation/tests/sync_equality.rs"]
mod sync_equality;
#[path = "prompt_registry_mutation/tests/sync_init_reload.rs"]
mod sync_init_reload;
