use betterlite_prompts::arguments::json::Object;

use crate::mcp_token_claims::Claims;

mod tests;

pub fn apply(
    claims: &mut Claims,
    add_claims: &Object,
    set_claims: &Object,
    remove_claims: &[String],
) {
    for (name, value) in add_claims {
        claims.entry(name.clone()).or_insert_with(|| value.clone());
    }
    for (name, value) in set_claims {
        claims.insert(name.clone(), value.clone());
    }
    for name in remove_claims {
        claims.shift_remove(name.as_str());
    }
}
