#![cfg(test)]
mod host;

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::Path;
use std::sync::Arc;

use rstest::rstest;
use tempfile::TempDir;
use tokio::runtime::{Builder, Handle, Runtime};
use uuid::Uuid;

use crate::sandbox_backend::bridge::SandboxBackendProtocol as _;
use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::protocol::{
    Edited, Encoding, ExecuteResponse, FileData, FileDownloadResponse, FileInfo,
    FileUploadResponse, GrepMatch, Listing, ReadResult, Reply, TransferError,
};
use crate::sandbox_backend::{Permissions, SandboxBackend};
use host::HostSandbox;

const SECRET: &str = "AWS_SECRET_ACCESS_KEY=leaked-from-host";
const WRITABLE: Permissions = Permissions {
    writable: true,
    allow_execute: true,
};
const READ_ONLY: Permissions = Permissions {
    writable: false,
    allow_execute: false,
};

fn backend_at(
    workdir: &str,
    runtime: Handle,
    permissions: Permissions,
) -> SandboxBackend<HostSandbox> {
    let sandbox = Arc::new(HostSandbox {
        workdir: workdir.to_owned(),
    });
    SandboxBackend::new(
        sandbox,
        runtime,
        permissions,
        Uuid::from_u128(0x0123_abcd_0000_4000_8000_0000_0000_0001),
    )
}

fn multi_thread() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

// a repo with links to a host secret, like a cloned repository can carry
fn repo_with_escape_links() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let host = dir.path().join("host_home");
    fs::create_dir_all(&host).unwrap();
    fs::write(host.join("credentials"), format!("{SECRET}\n")).unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "hello\n").unwrap();
    symlink(host.join("credentials"), repo.join("creds_link")).unwrap();
    symlink(&host, repo.join("home_link")).unwrap();
    let workdir = repo.to_str().unwrap().to_owned();
    (dir, workdir)
}

fn put(workdir: &str, path: &str) {
    let full = Path::new(workdir).join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, "x").unwrap();
}

fn on_disk(workdir: &str, path: &str) -> String {
    fs::read_to_string(Path::new(workdir).join(path)).unwrap()
}

#[rstest]
#[case::virtual_path("/src/a.py", "/work/src/a.py")]
#[case::relative_path("src/a.py", "/work/src/a.py")]
#[case::virtual_root("/", "/work")]
#[case::empty_path("", "/work")]
#[case::already_real("/work/src/a.py", "/work/src/a.py")]
#[case::real_root("/work", "/work")]
#[case::parent_at_root_stays_inside("/../etc/passwd", "/work/etc/passwd")]
#[case::parent_climbs_only_to_root("/src/../../etc", "/work/etc")]
#[case::doubled_slashes_and_dots("//src//./a.py", "/work/src/a.py")]
#[case::sibling_with_root_prefix("/workspace/a", "/work/workspace/a")]
fn to_real_maps_into_the_workdir(#[case] path: &str, #[case] real: &str) {
    let runtime = multi_thread();
    let backend = backend_at("/work/", runtime.handle().clone(), WRITABLE);
    assert_eq!(backend.to_real(path), real);
}

#[rstest]
#[case::root("/work", "/")]
#[case::inside("/work/src/a.py", "/src/a.py")]
#[case::sibling_with_root_prefix("/workspace/a", "/workspace/a")]
#[case::outside("/etc/passwd", "/etc/passwd")]
fn to_virtual_maps_the_workdir_to_slash(#[case] real: &str, #[case] path: &str) {
    let runtime = multi_thread();
    let backend = backend_at("/work", runtime.handle().clone(), WRITABLE);
    assert_eq!(backend.to_virtual(real), path);
}

#[test]
fn id_carries_the_session_prefix() {
    let runtime = multi_thread();
    let backend = backend_at("/work", runtime.handle().clone(), WRITABLE);
    assert_eq!(backend.id(), "litellm-harness-0123abcd");
}

#[tokio::test]
async fn grep_whole_repo_skips_symlinks_out_of_workspace() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let result = backend.agrep("AWS_SECRET", "/", None, None).await;
    assert_eq!(
        result,
        Reply::Done(Listing {
            matches: Vec::new(),
            truncated: false
        })
    );
}

#[tokio::test]
async fn grep_rooted_at_a_symlinked_dir_is_refused() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let result = backend.agrep("AWS_SECRET", "/home_link", None, None).await;
    assert_eq!(
        result,
        Reply::Error(
            "Path '/home_link': path resolves outside the workspace: /home_link".to_owned()
        )
    );
}

#[tokio::test]
async fn read_through_a_symlink_is_refused() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    assert_eq!(
        backend.aread("/creds_link", 0, 2000).await,
        ReadResult::Error(
            "Error reading file '/creds_link': path resolves outside the workspace: /creds_link"
                .to_owned()
        )
    );
}

#[tokio::test]
async fn glob_does_not_list_files_behind_symlinks() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let readme = FileInfo {
        path: "/README.md".to_owned(),
        is_dir: false,
    };
    assert_eq!(
        backend.aglob("**/*", "/").await,
        Reply::Done(Listing {
            matches: vec![readme],
            truncated: false
        })
    );
}

#[tokio::test]
async fn grep_still_finds_real_repo_files() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let hello = GrepMatch {
        path: "/README.md".to_owned(),
        line: 1,
        text: "hello".to_owned(),
    };
    assert_eq!(
        backend.agrep("hello", "", None, None).await,
        Reply::Done(Listing {
            matches: vec![hello],
            truncated: false
        })
    );
}

#[tokio::test]
async fn write_into_a_new_nested_directory_is_allowed() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let result = backend
        .awrite("/new_dir/sub/file.py", "print('hi')\r\n")
        .await;
    assert_eq!(result, Reply::Done("/new_dir/sub/file.py".to_owned()));
    assert_eq!(on_disk(&repo, "new_dir/sub/file.py"), "print('hi')\r\n");
}

#[tokio::test]
async fn write_under_a_symlinked_dir_is_refused() {
    let (dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let result = backend.awrite("/home_link/new_dir/evil.txt", "x").await;
    assert_eq!(
        result,
        Reply::Error("Error writing file '/home_link/new_dir/evil.txt': path resolves outside the workspace: /home_link/new_dir/evil.txt".to_owned())
    );
    assert!(!dir.path().join("host_home/new_dir").exists());
}

#[tokio::test]
async fn ls_lists_entries_sorted_with_dirs_marked() {
    let (_dir, repo) = repo_with_escape_links();
    fs::create_dir_all(Path::new(&repo).join("src")).unwrap();
    fs::write(Path::new(&repo).join("src/.env"), "x").unwrap();
    fs::write(Path::new(&repo).join("src/main.rs"), "x").unwrap();
    fs::create_dir_all(Path::new(&repo).join("src/lib")).unwrap();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let entries = [
        ("/src/.env", false),
        ("/src/lib/", true),
        ("/src/main.rs", false),
    ]
    .map(|(path, is_dir)| FileInfo {
        path: path.to_owned(),
        is_dir,
    });
    assert_eq!(backend.als("/src").await, Reply::Done(entries.to_vec()));
}

#[rstest]
#[case::missing("/nope", "Path '/nope': path_not_found")]
#[case::file("/README.md", "Path '/README.md': not_a_directory")]
#[case::outside(
    "/home_link",
    "Path '/home_link': path resolves outside the workspace: /home_link"
)]
#[tokio::test]
async fn ls_names_why_it_cannot_list(#[case] path: &str, #[case] message: &str) {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    assert_eq!(backend.als(path).await, Reply::Error(message.to_owned()));
}

#[tokio::test]
async fn read_returns_binary_as_base64_and_text_as_a_window() {
    let (_dir, repo) = repo_with_escape_links();
    fs::write(Path::new(&repo).join("blob.bin"), [0xff_u8, 0x00, 0x41]).unwrap();
    fs::write(Path::new(&repo).join("notes.txt"), "one\r\ntwo\nthree\n").unwrap();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    assert_eq!(
        backend.aread("/blob.bin", 0, 2000).await,
        ReadResult::Whole(FileData {
            content: "/wBB".to_owned(),
            encoding: Encoding::Base64
        })
    );
    let ReadResult::Window { file_data, window } = backend.aread("/notes.txt", 1, 1).await else {
        panic!("expected a window");
    };
    assert_eq!(file_data.content, "two\n");
    assert_eq!(
        (
            window.total_lines,
            window.start_line,
            window.end_line,
            window.next_offset
        ),
        (3, 2, 2, Some(2))
    );
    assert_eq!(
        backend.aread("/missing.txt", 0, 2000).await,
        ReadResult::Error("File '/missing.txt' not found".to_owned())
    );
}

#[tokio::test]
async fn read_only_sessions_refuse_every_change() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let refused = "Error: this session is read-only; files cannot be changed".to_owned();
    assert_eq!(
        backend.awrite("/a.txt", "x").await,
        Reply::Error(refused.clone())
    );
    assert_eq!(
        backend.aedit("/README.md", "hello", "bye", false).await,
        Reply::Error(refused.clone())
    );
    assert_eq!(backend.adelete("/README.md").await, Reply::Error(refused));
    let upload = backend
        .aupload_files(&[("/a.txt".to_owned(), b"x".to_vec())])
        .await;
    assert_eq!(
        upload,
        vec![FileUploadResponse {
            path: "/a.txt".to_owned(),
            outcome: Err(TransferError::PermissionDenied),
        }]
    );
    assert_eq!(on_disk(&repo, "README.md"), "hello\n");
    assert!(!Path::new(&repo).join("a.txt").exists());
}

#[tokio::test]
async fn edit_normalizes_crlf_and_counts_occurrences() {
    let (_dir, repo) = repo_with_escape_links();
    fs::write(Path::new(&repo).join("app.py"), "x = 1\r\ny = 1\r\n").unwrap();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let edited = Edited {
        path: "/app.py".to_owned(),
        occurrences: 2,
    };
    assert_eq!(
        backend.aedit("/app.py", " 1\r\n", " 2\r\n", true).await,
        Reply::Done(edited)
    );
    assert_eq!(on_disk(&repo, "app.py"), "x = 2\ny = 2\n");
}

#[rstest]
#[case::missing("/missing.py", "x", "Error: File '/missing.py' not found")]
#[case::outside(
    "/creds_link",
    "AWS",
    "Error editing file '/creds_link': path resolves outside the workspace: /creds_link"
)]
#[case::not_text(
    "/blob.bin",
    "A",
    "Error editing file '/blob.bin': the file is not UTF-8 text"
)]
#[case::not_found_in_file("/README.md", "absent", "Error: String not found in file: 'absent'")]
#[tokio::test]
async fn edit_names_why_it_changed_nothing(
    #[case] path: &str,
    #[case] old: &str,
    #[case] message: &str,
) {
    let (_dir, repo) = repo_with_escape_links();
    fs::write(Path::new(&repo).join("blob.bin"), [0xff_u8, 0x41]).unwrap();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    assert_eq!(
        backend.aedit(path, old, "new", false).await,
        Reply::Error(message.to_owned())
    );
    assert_eq!(
        fs::read(Path::new(&repo).join("blob.bin")).unwrap(),
        [0xff_u8, 0x41]
    );
}

#[tokio::test]
async fn delete_removes_files_and_refuses_the_root() {
    let (_dir, repo) = repo_with_escape_links();
    fs::create_dir_all(Path::new(&repo).join("build")).unwrap();
    fs::write(Path::new(&repo).join("build/out.o"), "x").unwrap();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    assert_eq!(
        backend.adelete("/build").await,
        Reply::Done("/build".to_owned())
    );
    assert!(!Path::new(&repo).join("build").exists());
    assert_eq!(
        backend.adelete("/").await,
        Reply::Error("Error: refusing to delete the workspace root".to_owned())
    );
    assert_eq!(
        backend.adelete("/../..").await,
        Reply::Error("Error: refusing to delete the workspace root".to_owned())
    );
    assert_eq!(
        backend.adelete("/build").await,
        Reply::Error("Error: '/build' not found".to_owned())
    );
    assert_eq!(on_disk(&repo, "README.md"), "hello\n");
}

#[tokio::test]
async fn delete_removes_a_link_but_not_its_target() {
    let (dir, repo) = repo_with_escape_links();
    symlink(
        Path::new(&repo).join("missing"),
        Path::new(&repo).join("dangling"),
    )
    .unwrap();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    assert_eq!(
        backend.adelete("/creds_link").await,
        Reply::Error(
            "Error deleting '/creds_link': path resolves outside the workspace: /creds_link"
                .to_owned()
        )
    );
    assert_eq!(
        backend.adelete("/dangling").await,
        Reply::Done("/dangling".to_owned())
    );
    assert!(dir.path().join("host_home/credentials").exists());
}

#[tokio::test]
async fn glob_skips_snapshot_dirs_and_matches_relative_to_its_root() {
    let (_dir, repo) = repo_with_escape_links();
    put(&repo, "src/a.py");
    put(&repo, "src/deep/b.py");
    put(&repo, "src/c.rs");
    put(&repo, ".git/hooks/x.py");
    put(&repo, "node_modules/m/i.py");
    put(&repo, "venv/v.py");
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let files = ["/src/a.py", "/src/deep/b.py"].map(|path| FileInfo {
        path: path.to_owned(),
        is_dir: false,
    });
    let expected = Reply::Done(Listing {
        matches: files.to_vec(),
        truncated: false,
    });
    assert_eq!(backend.aglob("*.py", "/").await, expected);
    assert_eq!(backend.aglob("**/*.py", "/src").await, expected);
    assert_eq!(
        backend.aglob("../*.py", "/").await,
        Reply::Error("Path traversal not allowed in glob pattern '../*.py'".to_owned())
    );
}

#[tokio::test]
async fn glob_marks_partial_results_when_find_fails() {
    let (_dir, repo) = repo_with_escape_links();
    let locked = Path::new(&repo).join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("hidden.md"), "x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let result = backend.aglob("*.md", "/").await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let readme = FileInfo {
        path: "/README.md".to_owned(),
        is_dir: false,
    };
    assert_eq!(
        result,
        Reply::Done(Listing {
            matches: vec![readme],
            truncated: true,
        })
    );
}

#[tokio::test]
async fn grep_filters_by_include_glob_and_caps_the_count() {
    let (_dir, repo) = repo_with_escape_links();
    fs::create_dir_all(Path::new(&repo).join("sub")).unwrap();
    fs::write(Path::new(&repo).join("a.txt"), "hello: one\nabc\n").unwrap();
    fs::write(
        Path::new(&repo).join("sub/c.txt"),
        "say hello\nhello again\n",
    )
    .unwrap();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let Reply::Done(found) = backend.agrep("hello", "/", Some("*.txt"), None).await else {
        panic!("grep failed");
    };
    let mut lines: Vec<(String, usize, String)> = found
        .matches
        .into_iter()
        .map(|grepped| (grepped.path, grepped.line, grepped.text))
        .collect();
    lines.sort();
    let expected = [
        ("/a.txt", 1, "hello: one"),
        ("/sub/c.txt", 1, "say hello"),
        ("/sub/c.txt", 2, "hello again"),
    ]
    .map(|(path, line, text)| (path.to_owned(), line, text.to_owned()));
    assert_eq!(lines, expected.to_vec());
    let Reply::Done(capped) = backend.agrep("hello", "/sub", None, Some(1)).await else {
        panic!("grep failed");
    };
    assert_eq!((capped.matches.len(), capped.truncated), (1, true));
    let fixed = backend.agrep("a.c", "/", None, None).await;
    assert_eq!(
        fixed,
        Reply::Done(Listing {
            matches: Vec::new(),
            truncated: false
        })
    );
}

#[tokio::test]
async fn grep_reports_a_failing_search_with_its_stderr() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), READ_ONLY);
    let Reply::Error(message) = backend.agrep("x", "/nope", None, None).await else {
        panic!("expected an error");
    };
    assert!(message.starts_with("Path '/nope': find: "), "{message}");
    assert_eq!(
        backend.agrep("x", "/", Some("../*"), None).await,
        Reply::Error("Path '/': Path traversal not allowed in glob pattern '../*'".to_owned())
    );
}

#[tokio::test]
async fn upload_and_download_report_each_path() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let files = [
        ("/data/a.bin", b"\x00\x01".as_slice()),
        ("/home_link/x", b"x".as_slice()),
    ]
    .map(|(path, data)| (path.to_owned(), data.to_vec()));
    let uploaded = backend.aupload_files(&files).await;
    let outcomes: Vec<Result<(), TransferError>> = uploaded
        .into_iter()
        .map(|response| response.outcome)
        .collect();
    assert_eq!(outcomes, vec![Ok(()), Err(TransferError::InvalidPath)]);
    let paths = ["/data/a.bin", "/missing", "/creds_link"].map(str::to_owned);
    let downloaded = backend.adownload_files(&paths).await;
    let expected = [
        ("/data/a.bin", Ok(vec![0_u8, 1])),
        ("/missing", Err(TransferError::FileNotFound)),
        ("/creds_link", Err(TransferError::InvalidPath)),
    ]
    .map(|(path, content)| FileDownloadResponse {
        path: path.to_owned(),
        content,
    });
    assert_eq!(downloaded, expected.to_vec());
}

#[rstest]
#[case::stdout_then_stderr("echo out; echo err >&2; exit 7", "out\n\nerr\n", 7_i32)]
#[case::stderr_only("echo err >&2", "err\n", 0_i32)]
#[case::runs_in_the_workdir("cat README.md", "hello\n", 0_i32)]
#[tokio::test]
async fn execute_merges_stderr_into_stdout(
    #[case] command: &str,
    #[case] output: &str,
    #[case] exit_code: i32,
) {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let expected = ExecuteResponse {
        output: output.to_owned(),
        exit_code,
        truncated: false,
    };
    assert_eq!(backend.aexecute(command, None).await, expected);
}

#[tokio::test]
async fn execute_truncates_on_a_character_boundary() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let response = backend
        .aexecute("printf a; yes \u{e9} | head -n 60000 | tr -d '\\n'", None)
        .await;
    assert_eq!(
        (
            response.output.len(),
            response.truncated,
            response.exit_code
        ),
        (99_999, true, 0_i32)
    );
}

#[tokio::test]
async fn execute_maps_a_sandbox_failure_to_exit_124() {
    let (_dir, repo) = repo_with_escape_links();
    let backend = backend_at(&repo, Handle::current(), WRITABLE);
    let response = backend.aexecute("exec sleep 3", Some(1)).await;
    assert_eq!((response.exit_code, response.truncated), (124_i32, false));
    assert!(
        response.output.starts_with("Error: the sandbox failed: "),
        "{}",
        response.output
    );
}

#[tokio::test]
async fn execute_is_refused_when_disabled() {
    let (_dir, repo) = repo_with_escape_links();
    let permissions = Permissions {
        writable: true,
        allow_execute: false,
    };
    let backend = backend_at(&repo, Handle::current(), permissions);
    let refused = ExecuteResponse {
        output: "Error: shell execution is disabled for this session".to_owned(),
        exit_code: 1,
        truncated: false,
    };
    assert_eq!(backend.aexecute("touch made", None).await, refused);
    assert!(!Path::new(&repo).join("made").exists());
}

#[test]
fn sync_methods_run_from_a_plain_thread() {
    let (_dir, repo) = repo_with_escape_links();
    let runtime = multi_thread();
    let backend = backend_at(&repo, runtime.handle().clone(), WRITABLE);
    assert_eq!(
        backend.write("/b.txt", "bee").unwrap(),
        Reply::Done("/b.txt".to_owned())
    );
    assert_eq!(backend.execute("cat b.txt", None).unwrap().output, "bee");
    assert_eq!(backend.ls("/").unwrap(), runtime.block_on(backend.als("/")));
}

#[test]
fn sync_methods_refuse_the_event_loop_thread() {
    let (_dir, repo) = repo_with_escape_links();
    let runtime = multi_thread();
    let backend = backend_at(&repo, runtime.handle().clone(), WRITABLE);
    let refused = runtime.block_on(async { backend.write("/b.txt", "bee") });
    let Err(error) = refused else {
        panic!("expected a refusal");
    };
    assert!(matches!(error, BackendError::EventLoopThread));
    assert_eq!(
        error.to_string(),
        "SandboxBackend sync methods cannot run on the event loop thread"
    );
    assert!(!Path::new(&repo).join("b.txt").exists());
}
