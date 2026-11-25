use rstest::rstest;

use super::super::content_type_for_file_id;

#[rstest]
#[case("file-1.png", "image/png")]
#[case("FILE.PNG", "image/png")]
#[case("report.png.txt", "image/png")]
#[case("abcpng", "image/png")]
#[case("photo.jpg", "image/jpeg")]
#[case("photo.jpeg", "image/jpeg")]
#[case("FILE.JPEG", "image/jpeg")]
#[case("anim.gif", "image/gif")]
#[case("data.csv", "text/csv")]
#[case("payload.json", "application/json")]
#[case("notes.txt", "text/plain")]
#[case("doc.pdf", "application/pdf")]
#[case("csvdata", "application/octet-stream")]
#[case("no-extension", "application/octet-stream")]
#[case("", "application/octet-stream")]
fn content_type_matches_liteellm_order(#[case] file_id: &str, #[case] expected: &str) {
    assert_eq!(content_type_for_file_id(file_id), expected);
}
