use subtle::ConstantTimeEq as _;

// Port of spend_tracking_utils._is_master_key: a digest never matches a raw key.
#[must_use]
pub fn matches_raw(presented: Option<&str>, master_key: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (presented, master_key) else {
        return false;
    };
    // secrets.compare_digest raises on non-ASCII str operands in Python; here it is false
    if !left.is_ascii() {
        return false;
    }
    if !right.is_ascii() {
        return false;
    }
    bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

mod tests;
