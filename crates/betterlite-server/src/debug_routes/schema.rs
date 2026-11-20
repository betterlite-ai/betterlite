use betterlite_openapi::document_builder::RouteDoc;

mod tests;

// LiteLLM hides all but /debug/asyncio-tasks; betterlite hides it too (docs/deviations/B4174.yaml)
pub const DOCUMENTED_ROUTES: &[RouteDoc] = &[];
