pub const SESSION_LANG: &str = "python";
pub const SANDBOX_ROOT: &str = "/sandbox";
pub const SANDBOX_PREFIX: &str = "/sandbox/";
pub const REQUIREMENTS_FILE: &str = ".litellm_requirements.txt";
pub const SKILL_REQUIREMENTS_FILE: &str = "requirements.txt";

// the listing runs inside the container, so the extension filter and the walk live here
pub const LISTING_CODE: &str = "
import os
import json
files = []
for root, dirs, filenames in os.walk('/sandbox'):
    for f in filenames:
        if f.endswith(('.gif', '.png', '.jpg', '.jpeg', '.pdf', '.csv', '.json')):
            files.append(os.path.join(root, f))
print(json.dumps(files))
";

pub fn pip_install_code(filename: &str) -> String {
    format!(
        "
import subprocess
import sys
subprocess.run(
    [sys.executable, '-m', 'pip', 'install', '-r', '{filename}'],
    check=True,
    cwd='/sandbox',
)
"
    )
}

pub fn wrapped_code(code: &str) -> String {
    format!(
        "
import os
os.chdir('/sandbox')
import sys
sys.path.insert(0, '/sandbox')

{code}
"
    )
}

pub fn mime_type(filename: &str) -> &'static str {
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "gif" => "image/gif",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "json" => "application/json",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}
