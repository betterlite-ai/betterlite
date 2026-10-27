use super::errors::DbError;

#[test]
fn db_error_is_typed() {
    assert_eq!(DbError::NoDbConnected.to_string(), "No DB Connected");
}
