use serde::Deserialize;

pub(crate) const MAX_OPERATIONS: usize = 256;
pub(crate) const MAX_FRAMES: usize = 64;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderDocument {
    pub(crate) requires_session_configuration: bool,
    pub(crate) fail_transform: bool,
    pub(crate) dropped: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Operation {
    Deferred,
    ShouldBuffer { text: String },
    Buffer { text: String },
    Flush,
    FlushWithArrival { text: String },
    Complete { value: bool },
    Flushing { value: bool },
}

impl Operation {
    fn size(&self) -> usize {
        match self {
            Self::ShouldBuffer { text }
            | Self::Buffer { text }
            | Self::FlushWithArrival { text } => text.len(),
            Self::Deferred | Self::Flush | Self::Complete { .. } | Self::Flushing { .. } => 0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Input {
    pub(crate) provider: Option<ProviderDocument>,
    pub(crate) backend_failures: Vec<usize>,
    pub(crate) operations: Vec<Operation>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        let mut sizes = self
            .operations
            .iter()
            .map(Operation::size)
            .collect::<Vec<_>>();
        if let Some(provider) = &self.provider {
            sizes.extend(provider.dropped.iter().map(String::len));
        }
        if self.operations.len() > MAX_OPERATIONS
            || self.backend_failures.len() > MAX_FRAMES
            || sizes.iter().any(|size| *size > MAX_TEXT_BYTES)
            || sizes.iter().copied().sum::<usize>() > MAX_CAPTURE_BYTES
        {
            return Err(String::from(
                "the scripted setup buffer input is past its limits",
            ));
        }
        Ok(())
    }
}
