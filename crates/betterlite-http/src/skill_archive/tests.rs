#![cfg(test)]

use std::fs;
use std::io::{Cursor, Read as _, Write as _};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
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

#[test]
fn skill_archive_rejects_malformed_zip_bytes() {
    assert!(build_skill_archive(b"not a ZIP archive").is_none());
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
    let body = vec![0_u8; 52_428_800];
    assert!(build_skill_archive(&upload(&[("SKILL.md", &body)])).is_some());
}

#[test]
fn skill_archive_rejects_one_byte_over_the_max_unpacked_bytes() {
    let body = vec![0_u8; 52_428_801];
    assert!(build_skill_archive(&upload(&[("SKILL.md", &body)])).is_none());
}

#[test]
fn skill_archive_rejects_duplicate_flattened_names() {
    assert!(
        build_skill_archive(&upload(&[
            ("skill/SKILL.md", b"body"),
            ("skill/a.txt", b"first"),
            ("skill//a.txt", b"second"),
        ]))
        .is_none()
    );
}

#[test]
fn skill_archive_rejects_uploads_with_no_common_root() {
    assert!(
        build_skill_archive(&upload(&[
            ("folderA/SKILL.md", b"body"),
            ("folderB/other.txt", b"data"),
        ]))
        .is_none()
    );
}

#[test]
fn skill_archive_accepts_exactly_the_max_entries() {
    let names: Vec<String> = (0_u32..999).map(|index| format!("x{index}.txt")).collect();
    let mut members = vec![("SKILL.md", b"body".as_slice())];
    members.extend(names.iter().map(|name| (name.as_str(), b"a".as_slice())));
    assert!(build_skill_archive(&upload(&members)).is_some());
}

#[test]
fn skill_archive_rejects_one_entry_over_the_max_entries() {
    let names: Vec<String> = (0_u32..1000).map(|index| format!("x{index}.txt")).collect();
    let mut members = vec![("SKILL.md", b"body".as_slice())];
    members.extend(names.iter().map(|name| (name.as_str(), b"a".as_slice())));
    assert!(build_skill_archive(&upload(&members)).is_none());
}

#[test]
fn skill_archive_rejects_a_zip_with_a_crc_mismatch() {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("SKILL.md", options).unwrap();
    writer.write_all(b"corrupt-me").unwrap();
    writer.finish().unwrap();
    let mut bytes = output.into_inner();
    let marker = b"corrupt-me";
    let position = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    bytes[position] ^= 0xFF;
    assert!(build_skill_archive(&bytes).is_none());
}

#[test]
fn skill_archive_tolerates_malformed_frontmatter() {
    let archive =
        build_skill_archive(&upload(&[("SKILL.md", b"---\nname: [\n---\nbody")])).unwrap();
    assert_eq!(archive.declared_name, None);
    assert_eq!(archive.declared_description, None);
}

#[test]
fn skill_archive_replaces_invalid_frontmatter_utf8() {
    let archive = build_skill_archive(&upload(&[(
        "SKILL.md",
        b"---\nname: valid\xFFname\n---\nbody",
    )]))
    .unwrap();
    assert_eq!(archive.declared_name, Some("valid\u{fffd}name".to_owned()));
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
fn skill_archive_requires_the_delimiter_at_the_very_start_of_the_manifest() {
    let archive =
        build_skill_archive(&upload(&[("SKILL.md", b" ---\nname: Leading\n---\nbody")])).unwrap();
    assert_eq!(archive.declared_name, None);
}

#[test]
fn skill_archive_ignores_a_frontmatter_field_that_is_not_a_string() {
    let archive = build_skill_archive(&upload(&[(
        "SKILL.md",
        b"---\nname: 123\ndescription: ok\n---\nbody",
    )]))
    .unwrap();
    assert_eq!(archive.declared_name, None);
    assert_eq!(archive.declared_description.as_deref(), Some("ok"));
}

#[test]
fn skill_archive_reads_a_bzip2_compressed_member() {
    let bytes = BASE64
        .decode(
            "UEsDBAoAAAAAAA6oYV30xqGHBwAAAAcAAAAOAAAAc2tpbGwvU0tJTEwubWQjIERlbW8KUEsDBC4AAAAMAA+oYV27FPsFegAAAIsAAAAOAAAAc2tpbGwvZGF0YS5iaW5CWmg5MUFZJlNZ4lLFawAAFNmAABBABRAAP+7fMCAAZBVQU9TJ+qbUG1GASqfqjTQBgAV9pwiWJ5qqs6KJywowhdZZhpaWS7vNndu1f2UedqK6tDs9NnKENFm7l9RKz5xxJhwIbuhBIwzUQhHjTYeZF3JFOFCQ4lLFa1BLAQI/AwoAAAAAAA6oYV30xqGHBwAAAAcAAAAOACQAAAAAAAAAIIC0gQAAAABza2lsbC9TS0lMTC5tZAoAIAAAAAAAAQAYAHnW6qM7at0BAAAAAAAAAAAAAAAAAAAAAFBLAQI/Ay4AAAAMAA+oYV27FPsFegAAAIsAAAAOACQAAAAAAAAAIIC0gTMAAABza2lsbC9kYXRhLmJpbgoAIAAAAAAAAQAYADVfLaQ7at0BAAAAAAAAAAAAAAAAAAAAAFBLBQYAAAAAAgACAMAAAADZAAAAAAA=",
        )
        .unwrap();
    let archive = build_skill_archive(&bytes).unwrap();
    let mut repacked = ZipArchive::new(Cursor::new(archive.content)).unwrap();
    let mut data = Vec::new();
    repacked
        .by_name("data.bin")
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    assert_eq!(
        data,
        b"bzip2 member payload for the skill archive oracle case, \
repeated for good measure. bzip2 member payload for the skill archive oracle case.\n"
            .as_slice()
    );
}

#[test]
fn skill_archive_reads_an_lzma_compressed_member() {
    let bytes = BASE64
        .decode(
            "UEsDBAoAAAAAAA6oYV30xqGHBwAAAAcAAAAOAAAAc2tpbGwvU0tJTEwubWQjIERlbW8KUEsDBD8AAgAOACKoYV0glgUcYQAAAIkAAAAOAAAAc2tpbGwvbHptYS5iaW4ZAQUAXQAQAAAANh6J3X1JUuD1HFWCmniukVUGa43emQjtMqsvL60n1NE32xcjT7Hb6CGbYINCs1O7GryCNtgNmaJDtY0OOwtggZqM4HxIzpBORGyho603TPfX///nglAAUEsBAj8DCgAAAAAADqhhXfTGoYcHAAAABwAAAA4AJAAAAAAAAAAggLSBAAAAAHNraWxsL1NLSUxMLm1kCgAgAAAAAAABABgAedbqoztq3QEAAAAAAAAAAAAAAAAAAAAAUEsBAj8DPwACAA4AIqhhXSCWBRxhAAAAiQAAAA4AJAAAAAAAAAAggLSBMwAAAHNraWxsL2x6bWEuYmluCgAgAAAAAAABABgA8vG0uDtq3QEAAAAAAAAAAAAAAAAAAAAAUEsFBgAAAAACAAIAwAAAAMAAAAAAAA==",
        )
        .unwrap();
    let archive = build_skill_archive(&bytes).unwrap();
    let mut repacked = ZipArchive::new(Cursor::new(archive.content)).unwrap();
    let mut data = Vec::new();
    repacked
        .by_name("lzma.bin")
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    assert_eq!(
        data,
        b"lzma member payload for the skill archive oracle case, \
repeated for good measure. lzma member payload for the skill archive oracle case.\n"
            .as_slice()
    );
}

#[test]
fn skill_archive_keeps_the_last_duplicate_frontmatter_key() {
    let archive = build_skill_archive(&upload(&[(
        "SKILL.md",
        b"---\nname: First\nname: Second\ndescription: demo\n---\nbody",
    )]))
    .unwrap();
    assert_eq!(archive.declared_name.as_deref(), Some("Second"));
}

#[test]
fn skill_archive_rejects_an_upload_with_a_trailing_backslash_member() {
    assert!(
        build_skill_archive(&upload(&[
            ("skill/SKILL.md", b"---\nname: Demo\n---\nbody"),
            ("skill/notes\\", b""),
        ]))
        .is_none()
    );
}

#[test]
fn skill_archive_rejects_a_local_and_central_header_name_mismatch() {
    let mut bytes = upload(&[("skill/SKILL.md", b"body")]);
    let marker = b"skill/SKILL.md";
    let mut occurrences = Vec::new();
    let mut start = 0;
    while let Some(position) = bytes[start..]
        .windows(marker.len())
        .position(|window| window == marker)
    {
        occurrences.push(start + position);
        start += position + 1;
    }
    assert_eq!(occurrences.len(), 2);
    let local_header_offset = occurrences[0];
    bytes[local_header_offset..local_header_offset + marker.len()]
        .copy_from_slice(b"skill/XKILL.md");
    assert!(build_skill_archive(&bytes).is_none());
}

#[test]
fn skill_archive_returns_none_for_a_corrupted_deflate_member() {
    let bytes = upload(&[("SKILL.md", b"some deflate-compressed body text")]);
    let marker = b"SKILL.md";
    let header_position = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    let data_start = header_position + marker.len();
    let mut corrupted = bytes;
    corrupted[data_start..data_start + 4].copy_from_slice(&[0xFF; 4]);
    assert!(build_skill_archive(&corrupted).is_none());
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
