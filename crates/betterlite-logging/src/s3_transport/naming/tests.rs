#![cfg(test)]

use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;

use crate::s3_transport::naming::{object_download_filename, object_key};
use crate::s3_transport::options::PartitionGranularity;
use crate::s3_transport::options::PartitionGranularity::{Day, Hour};

const LONG_X: &str = "5c0e0ea421571c300b5df6aec0a118b5c3dc02e0683a546341d5efc689df2f58";
const X_200: &str = "aa20c23e3201834050679e1d88941b9a6fed0557c9a705cb2c315e2e63fd486d";

enum Name<'case> {
    Key {
        path: &'case str,
        prefix: &'case str,
        file: &'case str,
        granularity: PartitionGranularity,
    },
    Download {
        response_id: &'case str,
    },
}

#[rstest]
#[case::short_key_with_hour(Name::Key { path: "bucket/path/", prefix: "team/", file: "a/b:c", granularity: Hour }, "bucket/path/team/2026-01-02/03/a_b_c.json")]
#[case::empty_path(Name::Key { path: "", prefix: "", file: "id", granularity: Day }, "2026-01-02/id.json")]
#[case::path_gets_a_slash(Name::Key { path: "logs", prefix: "", file: "id", granularity: Day }, "logs/2026-01-02/id.json")]
#[case::root_path(Name::Key { path: "/", prefix: "team/", file: "id", granularity: Day }, "/team/2026-01-02/id.json")]
#[case::exact_fit_is_kept(Name::Key { path: "", prefix: "", file: &"x".repeat(1008), granularity: Day }, &format!("2026-01-02/{}.json", "x".repeat(1008)))]
#[case::one_byte_over(Name::Key { path: "", prefix: "", file: &"x".repeat(1009), granularity: Day }, &format!("2026-01-02/{}_79ca9dd5b6c8126d19a8d49ddd910c25597c0cd1de0c5750b5eaa184044c1e2e.json", "x".repeat(64)))]
#[case::long_file_name(Name::Key { path: "", prefix: "", file: &"x".repeat(2000), granularity: Day }, &format!("2026-01-02/{}_{LONG_X}.json", "x".repeat(64)))]
#[case::long_file_name_keeps_prefix(Name::Key { path: "logs", prefix: "team/", file: &"x".repeat(2000), granularity: Hour }, &format!("logs/team/2026-01-02/03/{}_{LONG_X}.json", "x".repeat(64)))]
#[case::multibyte_file_name(Name::Key { path: "", prefix: "", file: &"\u{e9}".repeat(600), granularity: Day }, &format!("2026-01-02/{}_17b9cc826ac8cbc9eb90dc2da81df1cff7d8a0d79515f8818e165cecfe4c8885.json", "\u{e9}".repeat(32)))]
#[case::digest_names_the_unsanitized_file(Name::Key { path: "", prefix: "", file: &format!("a/b:{}", "x".repeat(1100)), granularity: Hour }, &format!("2026-01-02/03/a_b_{}_ef0129231d9a50d7111bed464af22535166cf0ffa371bedbbf4eb07bb013d8e2.json", "x".repeat(60)))]
#[case::prefix_at_minimum_keeps_only_the_digest(Name::Key { path: "", prefix: &"p".repeat(944), file: &"x".repeat(100), granularity: Day }, &format!("{}2026-01-02/09ecb6ebc8bcefc733f6f2ec44f791abeed6a99edf0cc31519637898aebd52d8.json", "p".repeat(944)))]
#[case::prefix_too_long(Name::Key { path: &"p".repeat(1000), prefix: "team/", file: &"x".repeat(200), granularity: Day }, &format!("{}/673b2f1d359040fd/2026-01-02/{X_200}.json", "p".repeat(926)))]
#[case::multibyte_prefix(Name::Key { path: "", prefix: &"\u{e9}".repeat(500), file: &"x".repeat(200), granularity: Day }, &format!("{}/8b671e21c36a4d79/2026-01-02/{X_200}.json", "\u{e9}".repeat(463)))]
#[case::prefix_cut_after_a_slash(Name::Key { path: "", prefix: &format!("{}/{}", "p".repeat(925), "q".repeat(100)), file: &"x".repeat(200), granularity: Day }, &format!("{}/dcc54efa1be155f4/2026-01-02/{X_200}.json", "p".repeat(925)))]
#[case::download_quote_and_slash_replaced(Name::Download { response_id: "a/b\"c" }, "time-2026-01-02T03-04-05-000006_a_b_c.json")]
#[case::download_exact_fit_is_kept(Name::Download { response_id: &"x".repeat(987) }, &format!("time-2026-01-02T03-04-05-000006_{}.json", "x".repeat(987)))]
#[case::download_one_byte_over(Name::Download { response_id: &"x".repeat(988) }, &format!("time-2026-01-02T03-04-05-000006_{}_0f977c7bdc26c9016d24d7f0d4a1d516e83b869f81d281add299030b647ce4e3.json", "x".repeat(32)))]
#[case::download_multibyte_response_id(Name::Download { response_id: &"\u{e9}".repeat(600) }, &format!("time-2026-01-02T03-04-05-000006_{}_d3a0fcee3b6adcf46fed0362778ea64025cb15dc6fffae4b1266114819f1beb2.json", "\u{e9}".repeat(16)))]
#[case::download_digest_names_the_unsanitized_id(Name::Download { response_id: &"/".repeat(988) }, &format!("time-2026-01-02T03-04-05-000006{}_e6eb0419fcc0d824b3bfa65af70b72b0a1371e1fbb158997543cb42d861be25b.json", "_".repeat(33)))]
fn s3_object_naming_behaviors_behavior(#[case] name: Name<'_>, #[case] expected: &str) {
    let start_time: NaiveDateTime = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_nano_opt(3, 4, 5, 6_999)
        .unwrap();
    let built = match name {
        Name::Key {
            path,
            prefix,
            file,
            granularity,
        } => object_key(path, prefix, start_time, file, granularity),
        Name::Download { response_id } => object_download_filename(start_time, response_id),
    };
    assert_eq!(built, expected);
    assert!(built.len() <= 1024);
    assert!(built.chars().map(char::len_utf8).sum::<usize>() <= 1024);
}
