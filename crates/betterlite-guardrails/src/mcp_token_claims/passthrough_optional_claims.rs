use crate::mcp_token_claims::Claims;

mod tests;

pub fn apply(claims: &mut Claims, optional_claims: &[String], incoming_claims: Option<&Claims>) {
    let Some(incoming) = incoming_claims else {
        return;
    };
    for name in optional_claims {
        if let Some(value) = incoming.get(name.as_str()) {
            claims.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
}
