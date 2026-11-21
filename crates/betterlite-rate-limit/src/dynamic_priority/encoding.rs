mod tests;

// _is_latin1_encodable: a value the proxy writes into a header carries Latin-1 code points only
#[must_use]
pub fn is_latin1_encodable(value: &str) -> bool {
    for character in value.chars() {
        if u32::from(character) >= 0x100 {
            return false;
        }
    }
    true
}
