use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct ConfigurationUpdate {
    pub name: String,
    pub value: SecretString,
}

#[derive(Debug, Clone)]
pub struct WriterEnvResult {
    pub replaced: bool,
    pub updates: Vec<ConfigurationUpdate>,
}
