use jsonwebtoken::Algorithm;

// JHWTHandler.SUPPORTED_JWT_ALGORITHMS: the ten asymmetric names; HS* and none never reach a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JwtAlgorithm {
    Rs256,
    Rs384,
    Rs512,
    Ps256,
    Ps384,
    Ps512,
    Es256,
    Es384,
    Es512,
    EdDsa,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Family {
    Rsa,
    Ec,
    Ed,
}

impl JwtAlgorithm {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "RS256" => Some(Self::Rs256),
            "RS384" => Some(Self::Rs384),
            "RS512" => Some(Self::Rs512),
            "PS256" => Some(Self::Ps256),
            "PS384" => Some(Self::Ps384),
            "PS512" => Some(Self::Ps512),
            "ES256" => Some(Self::Es256),
            "ES384" => Some(Self::Es384),
            "ES512" => Some(Self::Es512),
            "EdDSA" => Some(Self::EdDsa),
            _ => None,
        }
    }

    #[must_use]
    pub const fn family(self) -> Family {
        match self {
            Self::Rs256 | Self::Rs384 | Self::Rs512 | Self::Ps256 | Self::Ps384 | Self::Ps512 => {
                Family::Rsa
            }
            Self::Es256 | Self::Es384 | Self::Es512 => Family::Ec,
            Self::EdDsa => Family::Ed,
        }
    }

    // the SEC1 curve a JWK or certificate key must name for this algorithm
    #[must_use]
    pub const fn curve(self) -> Option<&'static str> {
        match self {
            Self::Es256 => Some("P-256"),
            Self::Es384 => Some("P-384"),
            Self::Es512 => Some("P-521"),
            Self::Rs256
            | Self::Rs384
            | Self::Rs512
            | Self::Ps256
            | Self::Ps384
            | Self::Ps512
            | Self::EdDsa => None,
        }
    }

    // jsonwebtoken has no ES512 verifier, so that one algorithm is None and goes to p521
    #[must_use]
    pub const fn library(self) -> Option<Algorithm> {
        match self {
            Self::Rs256 => Some(Algorithm::RS256),
            Self::Rs384 => Some(Algorithm::RS384),
            Self::Rs512 => Some(Algorithm::RS512),
            Self::Ps256 => Some(Algorithm::PS256),
            Self::Ps384 => Some(Algorithm::PS384),
            Self::Ps512 => Some(Algorithm::PS512),
            Self::Es256 => Some(Algorithm::ES256),
            Self::Es384 => Some(Algorithm::ES384),
            Self::Es512 => None,
            Self::EdDsa => Some(Algorithm::EdDSA),
        }
    }
}
