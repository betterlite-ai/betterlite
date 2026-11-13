#[cfg(unix)]
fn executable(directory: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = directory.join(name);
    fs::write(&path, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn path_resolver_finds_an_executable_on_the_search_path() {
    let directory = temp_dir();
    let binary = executable(directory.path(), "codex");
    let resolver = PathResolver {
        search_path: OsString::from(directory.path()),
        current_dir: directory.path().to_path_buf(),
    };
    assert_eq!(
        resolver.resolve("codex"),
        Some(binary.to_str().unwrap().to_owned())
    );
}

#[cfg(unix)]
#[test]
fn path_resolver_reports_a_missing_binary() {
    let directory = temp_dir();
    let resolver = PathResolver {
        search_path: OsString::from(directory.path()),
        current_dir: directory.path().to_path_buf(),
    };
    assert_eq!(resolver.resolve("codex"), None);
}

#[cfg(unix)]
#[test]
fn path_resolver_ignores_a_file_that_is_not_executable() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = temp_dir();
    let path = directory.path().join("codex");
    fs::write(&path, b"not a program").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let resolver = PathResolver {
        search_path: OsString::from(directory.path()),
        current_dir: directory.path().to_path_buf(),
    };
    assert_eq!(resolver.resolve("codex"), None);
}

#[cfg(unix)]
#[test]
fn path_resolver_resolves_an_absolute_command() {
    let directory = temp_dir();
    let binary = executable(directory.path(), "codex");
    let resolver = PathResolver {
        search_path: OsString::from("/nonexistent"),
        current_dir: directory.path().to_path_buf(),
    };
    assert_eq!(
        resolver.resolve(binary.to_str().unwrap()),
        Some(binary.to_str().unwrap().to_owned())
    );
}

#[cfg(unix)]
#[test]
fn path_resolver_resolves_a_relative_command_against_the_current_directory() {
    let directory = temp_dir();
    fs::create_dir_all(directory.path().join("bin")).unwrap();
    let binary = executable(&directory.path().join("bin"), "codex");
    let resolver = PathResolver {
        search_path: OsString::from("/nonexistent"),
        current_dir: directory.path().to_path_buf(),
    };
    assert_eq!(
        resolver.resolve("bin/codex"),
        Some(binary.to_str().unwrap().to_owned())
    );
}
