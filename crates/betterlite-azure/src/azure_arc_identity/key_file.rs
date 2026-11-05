use secrecy::SecretString;

use super::error::ArcIdentityError;
use super::{ArcHost, ArcPlatform};

const LINUX_TOKEN_DIRECTORY: &str = "/var/opt/azcmagent/tokens";
const WINDOWS_TOKEN_SUFFIX: &str = "\\AzureConnectedMachineAgent\\Tokens";
const UNEXPANDED_PROGRAM_DATA: &str = "%ProgramData%";
const MAX_KEY_FILE_BYTES: u64 = 4096;

// Only the last path component of the challenge value is used, so the challenge cannot leave the token directory.
fn challenge_stem(windows: bool, challenge_value: &str) -> &str {
    let mut characters = challenge_value.chars();
    characters.next();
    let without_drive = if windows && characters.next() == Some(':') {
        characters.as_str()
    } else {
        challenge_value
    };
    let separators: &[char] = if windows { &['/', '\\'] } else { &['/'] };
    let name = without_drive.rsplit(separators).next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((stem, _extension)) if stem.chars().any(|character| character != '.') => stem,
        _ => name,
    }
}

pub(super) fn read_key_file<H>(
    host: &H,
    challenge_value: &str,
) -> Result<SecretString, ArcIdentityError>
where
    H: ArcHost,
{
    let (directory, separator) = match host.platform() {
        ArcPlatform::Linux => (LINUX_TOKEN_DIRECTORY.to_owned(), '/'),
        ArcPlatform::Windows => {
            let program_data = host
                .environment("ProgramData")
                .unwrap_or_else(|| UNEXPANDED_PROGRAM_DATA.to_owned());
            (format!("{program_data}{WINDOWS_TOKEN_SUFFIX}"), '\\')
        }
        ArcPlatform::Other(platform) => {
            return Err(ArcIdentityError::UnsupportedPlatform { platform });
        }
    };
    let stem = challenge_stem(separator == '\\', challenge_value);
    let path = format!("{directory}{separator}{stem}.key");
    let size = host.file_size(&path).map_err(ArcIdentityError::KeyFile)?;
    if size > MAX_KEY_FILE_BYTES {
        return Err(ArcIdentityError::KeyFileTooLarge);
    }
    let text = host.read_text(&path).map_err(ArcIdentityError::KeyFile)?;
    Ok(SecretString::from(
        text.replace("\r\n", "\n").replace('\r', "\n"),
    ))
}
