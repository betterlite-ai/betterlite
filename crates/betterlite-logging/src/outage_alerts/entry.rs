#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    ApiError { status_code: Option<i64> },
    Other,
}
