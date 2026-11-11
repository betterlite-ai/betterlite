use crate::non_default_params::Params;

pub struct FirstUnsupported(pub String);

pub fn drop_unsupported_params(
    non_default_params: &mut Params,
    supported: &[String],
    drop: bool,
) -> Result<Vec<String>, FirstUnsupported> {
    let unsupported: Vec<String> = non_default_params
        .keys()
        .filter(|key| !supported.contains(key))
        .cloned()
        .collect();
    if drop {
        for key in &unsupported {
            non_default_params.remove(key);
        }
        return Ok(unsupported);
    }
    unsupported
        .into_iter()
        .next()
        .map_or_else(|| Ok(Vec::new()), |key| Err(FirstUnsupported(key)))
}
