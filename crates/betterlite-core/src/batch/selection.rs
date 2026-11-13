use indexmap::IndexMap;

#[must_use]
pub fn merge_deployment_params<Params>(
    request_params: &IndexMap<String, Params>,
    deployment_params: &IndexMap<String, Params>,
    nested_request_params: &IndexMap<String, Params>,
) -> IndexMap<String, Params>
where
    Params: Clone,
{
    let mut merged = request_params.clone();
    for (key, value) in deployment_params {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in nested_request_params {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

pub fn first_ordered_model_result<Response, Error>(
    results: Vec<Result<Option<Response>, Error>>,
) -> Result<Option<Response>, Error> {
    for result in results {
        if let Some(response) = result? {
            return Ok(Some(response));
        }
    }
    Ok(None)
}

#[must_use]
pub fn all_responses<Response, Error>(
    results: Vec<Result<Option<Response>, Error>>,
) -> Vec<Response> {
    results
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect()
}
