mod tests;

// The judge saw A and B in coin order; the attempt row records real, shadow or tie
#[must_use]
pub fn unmask_preference(raw: &str, real_is_a: bool) -> String {
    let normalized = raw.trim().to_lowercase();
    let mapped = match normalized.as_str() {
        "a" if real_is_a => "real",
        "a" => "shadow",
        "b" if real_is_a => "shadow",
        "b" => "real",
        _ => "tie",
    };
    mapped.to_owned()
}
