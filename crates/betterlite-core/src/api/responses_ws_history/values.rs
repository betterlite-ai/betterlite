use betterlite_models::decisions::Json;
const SIGN_MASK: u64 = 0x7fff_ffff_ffff_ffff;

// LiteLLM branches on `if value:` here: empty containers, zero and empty text take the other path
#[must_use]
pub(super) fn has_content(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        // comparing the bits keeps NaN truthy and both zeroes falsey without a float comparison clippy denies
        Json::Number(number) => number
            .as_f64()
            .is_none_or(|float| (float.to_bits() & SIGN_MASK) != 0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
