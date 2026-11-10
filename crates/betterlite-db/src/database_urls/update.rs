#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationUpdate {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterEnvResult {
    pub replaced: bool,
    pub updates: Vec<ConfigurationUpdate>,
}
