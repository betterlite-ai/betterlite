use betterlite_models::api_base_lookup::ApiBaseError;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HangingRequestError {
    #[error("the {field} field must be a string")]
    FieldType { field: &'static str },
    #[error("the api base of the hanging request could not be resolved")]
    ApiBase(#[from] ApiBaseError),
}
