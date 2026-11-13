const OS_ENVIRON_PREFIX: &str = "os.environ/";

pub(super) fn resolve(secret_name: &str) -> String {
    match secret_name.strip_prefix(OS_ENVIRON_PREFIX) {
        Some(_) => secret_name.replace(OS_ENVIRON_PREFIX, ""),
        None => secret_name.to_owned(),
    }
}
