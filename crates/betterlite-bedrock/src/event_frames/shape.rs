use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamMember {
    Chunk,
    InternalServerException,
    ModelStreamErrorException,
    ValidationException,
    ThrottlingException,
    ModelTimeoutException,
    ServiceUnavailableException,
}

impl StreamMember {
    const ALL: [Self; 7] = [
        Self::Chunk,
        Self::InternalServerException,
        Self::ModelStreamErrorException,
        Self::ValidationException,
        Self::ThrottlingException,
        Self::ModelTimeoutException,
        Self::ServiceUnavailableException,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chunk => "chunk",
            Self::InternalServerException => "internalServerException",
            Self::ModelStreamErrorException => "modelStreamErrorException",
            Self::ValidationException => "validationException",
            Self::ThrottlingException => "throttlingException",
            Self::ModelTimeoutException => "modelTimeoutException",
            Self::ServiceUnavailableException => "serviceUnavailableException",
        }
    }

    // httpStatusCode of the member's error trait in botocore's bedrock-runtime service-2 ResponseStream
    #[must_use]
    pub const fn http_status(self) -> Option<u16> {
        match self {
            Self::Chunk => None,
            Self::InternalServerException => Some(500),
            Self::ModelStreamErrorException => Some(424),
            Self::ValidationException => Some(400),
            Self::ThrottlingException => Some(429),
            Self::ModelTimeoutException => Some(408),
            Self::ServiceUnavailableException => Some(503),
        }
    }
}

pub struct ResponseStreamShape {
    pub members: BTreeMap<&'static str, StreamMember>,
}

impl ResponseStreamShape {
    #[must_use]
    pub fn bedrock_runtime() -> Self {
        let mut members = BTreeMap::new();
        for member in StreamMember::ALL {
            members.insert(member.name(), member);
        }
        Self { members }
    }
}
