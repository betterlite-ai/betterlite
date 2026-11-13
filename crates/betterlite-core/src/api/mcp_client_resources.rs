#![expect(clippy::disallowed_types, reason = "MCP JSON passthrough")]

pub mod detail;
pub mod list;
pub mod operations;
pub mod session;
pub mod settings;
pub mod types;

mod failure;
mod pagination;

#[path = "mcp_client_resources/tests/cancellation_tests.rs"]
mod cancellation_tests;
#[path = "mcp_client_resources/tests/deadline_tests.rs"]
mod deadline_tests;
#[path = "mcp_client_resources/tests/fixture.rs"]
mod fixture;
#[path = "mcp_client_resources/tests/get_prompt_tests.rs"]
mod get_prompt_tests;
#[path = "mcp_client_resources/tests/list_prompts_tests.rs"]
mod list_prompts_tests;
#[path = "mcp_client_resources/tests/list_resources_tests.rs"]
mod list_resources_tests;
#[path = "mcp_client_resources/tests/list_templates_tests.rs"]
mod list_templates_tests;
#[path = "mcp_client_resources/tests/read_resource_tests.rs"]
mod read_resource_tests;
