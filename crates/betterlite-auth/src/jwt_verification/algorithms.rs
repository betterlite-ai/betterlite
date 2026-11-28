const ALL: [JwtAlgorithm; 10] = [
    JwtAlgorithm::Rs256,
    JwtAlgorithm::Rs384,
    JwtAlgorithm::Rs512,
    JwtAlgorithm::Ps256,
    JwtAlgorithm::Ps384,
    JwtAlgorithm::Ps512,
    JwtAlgorithm::Es256,
    JwtAlgorithm::Es384,
    JwtAlgorithm::Es512,
    JwtAlgorithm::EdDsa,
];

const CURVES: [(&str, &str); 3] = [("ES256", "P-256"), ("ES384", "P-384"), ("ES512", "P-521")];

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
        ALL.iter()
            .copied()
            .find(|algorithm| algorithm.wire_name() == name)
    }

    // the exact name a JWT header carries, which jsonwebtoken's own parser also knows
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Rs256 => "RS256",
            Self::Rs384 => "RS384",
            Self::Rs512 => "RS512",
            Self::Ps256 => "PS256",
            Self::Ps384 => "PS384",
            Self::Ps512 => "PS512",
            Self::Es256 => "ES256",
            Self::Es384 => "ES384",
            Self::Es512 => "ES512",
            Self::EdDsa => "EdDSA",
        }
    }

    #[must_use]
    pub const fn family(self) -> Family {
        if matches!(self, Self::Es256 | Self::Es384 | Self::Es512) {
            return Family::Ec;
        }
        if matches!(self, Self::EdDsa) {
            return Family::Ed;
        }
        Family::Rsa
    }

    // the SEC1 curve a JWK or certificate key must name for this algorithm
    #[must_use]
    pub fn curve(self) -> Option<&'static str> {
        CURVES
            .iter()
            .find(|(name, _)| *name == self.wire_name())
            .map(|(_, curve)| *curve)
    }
}
