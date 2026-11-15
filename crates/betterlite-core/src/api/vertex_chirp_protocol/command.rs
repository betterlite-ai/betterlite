use serde::Serialize;

#[derive(Serialize)]
pub(super) struct ConfigureCommand<'config> {
    pub(super) kind: CommandKind,
    pub(super) model: &'config str,
    pub(super) language_codes: [&'config str; 1],
    pub(super) sample_rate_hertz: i64,
}

#[derive(Serialize)]
pub(super) enum CommandKind {
    #[serde(rename = "configure")]
    Configure,
}
