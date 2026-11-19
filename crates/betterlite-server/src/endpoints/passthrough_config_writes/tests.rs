#![cfg(test)]

#[path = "tests/corpus.rs"]
mod corpus;

#[path = "tests/create_routes.rs"]
mod create_routes;

#[path = "tests/delete_routes.rs"]
mod delete_routes;

#[path = "tests/error_mapping.rs"]
mod error_mapping;

#[path = "tests/initialize.rs"]
mod initialize;

#[path = "tests/support.rs"]
mod support;

#[path = "tests/update_routes.rs"]
mod update_routes;
