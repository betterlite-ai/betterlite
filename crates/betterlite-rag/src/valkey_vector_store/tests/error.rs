use crate::valkey_vector_store::error::ValkeyError;

#[test]
fn create_calls_fail_with_the_search_only_message() {
    assert_eq!(
        ValkeyError::SearchOnly.to_string(),
        "Valkey vector store is search-only; create indexes with FT.CREATE directly"
    );
}

#[test]
fn port_range_names_the_value() {
    assert_eq!(
        ValkeyError::PortRange { value: 70000 }.to_string(),
        "valkey_port must be between 0 and 65535, got 70000"
    );
}
