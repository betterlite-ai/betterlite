#![cfg(test)]

// Pinned Postgres shapes for the managed-resource tables, copied from the migrations
pub const SCHEMA: &str = include_str!("fixtures/schema.sql");
