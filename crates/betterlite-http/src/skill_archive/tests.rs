#![cfg(test)]

use std::fs;
use std::io::{Cursor, Read as _, Write as _};
use std::path::Path;

use rstest::rstest;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::build_skill_archive;

fn upload(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in members {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

#[test]
fn skill_archive_builds_root_manifest_and_metadata() {
    let archive = build_skill_archive(&upload(&[
        (
            "skill/SKILL.md",
            b"---\nname:  Demo skill \ndescription: ' A demo ' \n---\nbody",
        ),
        ("skill/README.md", b"readme"),
    ]))
    .unwrap();
    assert_eq!(archive.declared_name.as_deref(), Some("Demo skill"));
    assert_eq!(archive.declared_description.as_deref(), Some("A demo"));
    assert!(archive.digest.starts_with("sha256:"));
    let mut repacked = ZipArchive::new(Cursor::new(archive.content)).unwrap();
    let mut manifest = Vec::new();
    repacked
        .by_name("SKILL.md")
        .unwrap()
        .read_to_end(&mut manifest)
        .unwrap();
    assert_eq!(
        manifest,
        b"---\nname:  Demo skill \ndescription: ' A demo ' \n---\nbody"
    );
    repacked.by_name("skill/SKILL.md").unwrap_err();
}

#[rstest]
#[case::empty(None)]
#[case::missing_manifest(Some("skill/README.md"))]
#[case::unsafe_path(Some("skill/../SKILL.md"))]
#[case::backslash(Some("skill\\SKILL.md"))]
#[case::absolute(Some("/SKILL.md"))]
#[case::drive_letter(Some("C:/SKILL.md"))]
fn skill_archive_rejects_invalid_uploads(#[case] path: Option<&str>) {
    let members = path
        .map(|member_path| vec![(member_path, b"body".as_slice())])
        .unwrap_or_default();
    assert!(build_skill_archive(&upload(&members)).is_none());
}

#[rstest]
#[case::null_byte("skill/ev\0il.txt")]
#[case::backslash("skill/evil\\file.txt")]
#[case::dot_segment("skill/./evil.txt")]
fn skill_archive_rejects_unsafe_sibling_members(#[case] unsafe_path: &str) {
    assert!(
        build_skill_archive(&upload(&[
            ("skill/SKILL.md", b"body"),
            (unsafe_path, b"data"),
        ]))
        .is_none()
    );
}

#[test]
fn skill_archive_accepts_exactly_the_max_unpacked_bytes() {
    let body = vec![0u8; 52_428_800];
    assert!(build_skill_archive(&upload(&[("SKILL.md", &body)])).is_some());
}

#[test]
fn skill_archive_rejects_one_byte_over_the_max_unpacked_bytes() {
    let body = vec![0u8; 52_428_801];
    assert!(build_skill_archive(&upload(&[("SKILL.md", &body)])).is_none());
}

#[test]
fn skill_archive_tolerates_malformed_frontmatter() {
    let archive =
        build_skill_archive(&upload(&[("SKILL.md", b"---\nname: [\n---\nbody")])).unwrap();
    assert_eq!(archive.declared_name, None);
    assert_eq!(archive.declared_description, None);
}

#[test]
fn skill_archive_digest_is_reproducible() {
    let first = build_skill_archive(&upload(&[("SKILL.md", b"body")])).unwrap();
    let second = build_skill_archive(&upload(&[("SKILL.md", b"body")])).unwrap();
    assert_eq!(first.content, second.content);
    assert_eq!(first.digest, second.digest);
}

#[test]
fn skill_archive_repacks_sorted_members_with_fixed_metadata() {
    let archive = build_skill_archive(&upload(&[
        ("skill/z.txt", b"z"),
        ("skill/SKILL.md", b"body"),
        ("skill/a.txt", b"a"),
    ]))
    .unwrap();
    let mut repacked = ZipArchive::new(Cursor::new(archive.content)).unwrap();
    let names = (0..repacked.len())
        .map(|index| {
            repacked
                .by_index(index)
                .unwrap()
                .name()
                .unwrap()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["SKILL.md", "a.txt", "z.txt"]);
    let mut assert_metadata = |index| {
        let entry = repacked.by_index(index).unwrap();
        assert_eq!(
            entry.last_modified(),
            Some(zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap())
        );
        assert_eq!(entry.unix_mode(), Some(0o644));
        assert_eq!(entry.compression(), CompressionMethod::Deflated);
    };
    assert_metadata(0);
    assert_metadata(1);
    assert_metadata(2);
}

#[test]
fn skill_archive_replays_the_skill_archive_zip_corpus() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/skill_archive_zip");
    fs::read_dir(path)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .for_each(|bytes| {
            drop(build_skill_archive(&bytes));
        });
}
