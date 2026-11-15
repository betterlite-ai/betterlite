use rstest::rstest;

use super::character::character_create_get;
use super::edit::{edit_prompt, unused_prefetched_source};
use super::extension::video_extension;
use super::media::{fallback_octet_stream, known_extension_mime, video_part_filename};

#[rstest]
#[case::character_create_get(1)]
#[case::video_edit_prompt(2)]
#[case::video_extension(3)]
#[case::video_part_filename(4)]
#[case::known_extension_mime(5)]
#[case::fallback_octet_stream(6)]
#[case::unused_prefetched_source(7)]
fn video_edits_cases(#[case] case: u8) {
    match case {
        1 => character_create_get(),
        2 => edit_prompt(),
        3 => video_extension(),
        4 => video_part_filename(),
        5 => known_extension_mime(),
        6 => fallback_octet_stream(),
        _ => unused_prefetched_source(),
    }
}
