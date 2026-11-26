pub trait ModelRewrite {
    fn set_request_model(&mut self, model: &str);

    fn refresh_parsed_body(&mut self);

    fn refresh_json_cache(&mut self);

    // orjson.dumps is the only fallible write, and it runs after the body snapshots are already changed
    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure>;

    fn set_path_model(&mut self, model: &str);
}

#[derive(Debug, thiserror::Error)]
#[error("the rewritten request body could not be serialized")]
pub struct RewriteFailure;

// _check_key_model_budget_with_fallback rewrites four request snapshots in a fixed order
pub fn rewrite_model(request: &mut dyn ModelRewrite, model: &str) -> Result<(), RewriteFailure> {
    request.set_request_model(model);
    request.refresh_parsed_body();
    request.refresh_json_cache();
    request.refresh_serialized_body()?;
    request.set_path_model(model);
    Ok(())
}
