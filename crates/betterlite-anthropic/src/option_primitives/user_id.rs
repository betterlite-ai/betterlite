use regex::Regex;

// Python's re.match lets a final newline stand before the end anchor, and its \d and \s cover Unicode
const EMAIL_PATTERN: &str = r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?:\n)?\z";
const PHONE_PATTERN: &str = r"^\+?[\d\p{White_Space}\x{1c}-\x{1f}()\x{2d}]{7,}(?:\n)?\z";

#[derive(Debug, thiserror::Error)]
pub enum UserIdError {
    #[error("a user id pattern does not compile")]
    Pattern(#[source] regex::Error),
}

pub struct UserIdPolicy {
    email: Regex,
    phone: Regex,
}

impl UserIdPolicy {
    pub fn compile() -> Result<Self, UserIdError> {
        Ok(Self {
            email: Regex::new(EMAIL_PATTERN).map_err(UserIdError::Pattern)?,
            phone: Regex::new(PHONE_PATTERN).map_err(UserIdError::Pattern)?,
        })
    }

    #[must_use]
    pub fn accepts(&self, user_id: &str) -> bool {
        if self.email.is_match(user_id) {
            return false;
        }
        !self.phone.is_match(user_id)
    }
}
