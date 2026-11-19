// The vector store search handlers: an HTTP provider gets a validated, transformed and signed request, and a direct store runs the search on its own datastore
pub mod config;
pub mod error;
pub mod openai;
mod params;
pub mod service;
mod tests;
