use crate::assume_role::{ArnIdentity, parse_role_arn};

type Row = (
    &'static str,
    Option<(&'static str, &'static str, &'static str)>,
);

const ROWS: [Row; 14] = [
    (
        "arn:aws:iam::123456789012:role/MyRole",
        Some(("aws", "123456789012", "MyRole")),
    ),
    (
        "arn:aws:iam::123456789012:role/path/to/MyRole",
        Some(("aws", "123456789012", "MyRole")),
    ),
    (
        "arn:aws:sts::123456789012:assumed-role/MyRole/session-name",
        Some(("aws", "123456789012", "MyRole")),
    ),
    (
        "arn:aws:sts::123456789012:assumed-role/MyRole",
        Some(("aws", "123456789012", "MyRole")),
    ),
    ("arn:aws-cn:iam::123:role/", Some(("aws-cn", "123", ""))),
    ("arn:aws:sts::123:assumed-role/", Some(("aws", "123", ""))),
    ("arn:aws:iam::123:role/a:b/c", Some(("aws", "123", "c"))),
    (
        "arn:p:s:r:not-digits:role/R",
        Some(("p", "not-digits", "R")),
    ),
    ("arn:aws:iam::123:user/bob", None),
    ("arn:aws:iam::123:rolex/MyRole", None),
    ("arn:aws:iam::123", None),
    ("ARN:aws:iam::123:role/MyRole", None),
    ("", None),
    ("arn:aws:iam::123:assumed-role", None),
];

pub(super) fn assumed_role_arn_matching() {
    for (arn, expected) in ROWS {
        let identity = expected.map(|(partition, account, role)| ArnIdentity {
            partition,
            account,
            role,
        });
        assert_eq!(parse_role_arn(arn), identity, "{arn}");
    }
}
