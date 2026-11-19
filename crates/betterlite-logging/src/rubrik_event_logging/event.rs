#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
pub enum EventKind {
    #[strum(serialize = "success")]
    Success,
    #[strum(serialize = "failure")]
    Failure,
}
