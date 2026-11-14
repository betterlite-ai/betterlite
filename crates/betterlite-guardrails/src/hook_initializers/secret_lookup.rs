use secrecy::SecretString;

pub trait SecretLookup {
    fn get_secret_str(&self, name: &str) -> Option<SecretString>;
}
