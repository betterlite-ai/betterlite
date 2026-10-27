use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub connect_timeout: u64,
    pub read_timeout: u64,
    pub retries: Retries,
    pub s3: S3Options,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            connect_timeout: 5,
            read_timeout: 30,
            retries: Retries::default(),
            s3: S3Options::default(),
        }
    }
}

impl Options {
    pub(super) fn validate(&self, paths: &mut Vec<String>) {
        if !(1..=86_400).contains(&self.connect_timeout) {
            paths.push("s3_config.connect_timeout".to_owned());
        }
        if !(1..=86_400).contains(&self.read_timeout) {
            paths.push("s3_config.read_timeout".to_owned());
        }
        if matches!(self.s3.addressing_style, AddressingStyle::Virtual) {
            paths.push("s3_config.s3.addressing_style".to_owned());
        }
        if self.retries.max_attempts == u32::MAX {
            paths.push("s3_config.retries.max_attempts".to_owned());
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Retries {
    pub max_attempts: u32,
}

impl Default for Retries {
    fn default() -> Self {
        Self { max_attempts: 2 }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct S3Options {
    pub addressing_style: AddressingStyle,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressingStyle {
    #[default]
    Auto,
    Path,
    Virtual,
}
