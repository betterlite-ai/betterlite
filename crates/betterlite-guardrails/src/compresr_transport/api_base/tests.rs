#![cfg(test)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use rstest::rstest;

use super::{parse_ip_literal, validate_api_base};
use crate::compresr_transport::error::CompresrTransportError;

fn v4(first: u8, second: u8, third: u8, fourth: u8) -> Option<IpAddr> {
    Some(IpAddr::V4(Ipv4Addr::new(first, second, third, fourth)))
}

fn metadata_ip() -> Option<IpAddr> {
    v4(169, 254, 169, 254)
}

#[rstest]
#[case::dotted_quad("169.254.169.254", metadata_ip())]
#[case::decimal_single_integer("2852039166", metadata_ip())]
#[case::hex_single_integer("0xa9fea9fe", metadata_ip())]
#[case::hex_uppercase("0XA9FEA9FE", metadata_ip())]
#[case::binary("0b1010", v4(0, 0, 0, 10))]
#[case::octal("0o10", v4(0, 0, 0, 8))]
#[case::underscored("1_000", v4(0, 0, 3, 232))]
#[case::whitespace(" 2852039166 ", metadata_ip())]
#[case::zero("0", v4(0, 0, 0, 0))]
#[case::mapped_ipv6("::ffff:169.254.169.254", metadata_ip())]
#[case::mapped_ipv6_hex("::ffff:a9fe:a9fe", metadata_ip())]
#[case::ipv6(
    "fd00:ec2::254",
    Some(IpAddr::V6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254)))
)]
#[case::wider_than_ipv4("4294967296", Some(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 1, 0, 0))))]
#[case::octal_dotted("0251.0376.0251.0376", None)]
#[case::leading_zero_decimal("0177", None)]
#[case::double_underscore("1__0", None)]
#[case::trailing_underscore("1_", None)]
#[case::negative("-1", None)]
#[case::explicit_plus("+5", v4(0, 0, 0, 5))]
#[case::doubled_plus("++5", None)]
#[case::hex_with_leading_zero("0x00ff", v4(0, 0, 0, 255))]
#[case::node_style("0x10.0x10.0x10.0x10", None)]
#[case::hostname("metadata.google.internal", None)]
#[case::empty("", None)]
#[case::too_large("340282366920938463463374607431768211456", None)]
fn ip_literals_parse_like_python(#[case] host: &str, #[case] expected: Option<IpAddr>) {
    assert_eq!(parse_ip_literal(host), expected);
}

#[rstest]
#[case::decimal_encoding("http://2852039166")]
#[case::hex_encoding("http://0xa9fea9fe")]
#[case::mapped_ipv6("http://[::ffff:169.254.169.254]")]
#[case::metadata_google("http://metadata.google.internal")]
#[case::metadata_goog("https://metadata.goog")]
#[case::metadata_azure("http://metadata.azure.com")]
#[case::metadata_azure_internal("http://metadata.azure.internal")]
#[case::metadata_address("http://169.254.169.254/latest/meta-data/")]
#[case::metadata_ipv6("http://[fd00:ec2::254]")]
#[case::alibaba_metadata("http://100.100.100.200")]
#[case::azure_metadata("http://168.63.129.16")]
#[case::uppercase_host("http://METADATA.GOOGLE.INTERNAL")]
#[case::dotted_octal_parts("http://0251.0376.0251.0376")]
#[case::dotted_hex_parts("http://0xa9.0xfe.0xa9.0xfe")]
fn api_base_rejects_cloud_metadata_targets(#[case] url: &str) {
    assert!(matches!(
        validate_api_base(url),
        Err(CompresrTransportError::BlockedHost { .. })
    ));
}

#[rstest]
#[case::private_ten("http://10.0.0.5:8080")]
#[case::private_172("https://172.16.31.9")]
#[case::private_192("http://192.168.0.1/compresr")]
#[case::loopback("http://127.0.0.1:9000")]
#[case::public_host("https://api.compresr.ai")]
#[case::uppercase_scheme("HTTPS://api.compresr.ai")]
fn api_base_accepts_private_and_public_targets(#[case] url: &str) {
    assert_eq!(validate_api_base(url), Ok(url.to_owned()));
}

#[rstest]
#[case::ftp("ftp://api.compresr.ai", CompresrTransportError::Scheme)]
#[case::file("file:///etc/passwd", CompresrTransportError::Scheme)]
#[case::no_scheme("api.compresr.ai", CompresrTransportError::Scheme)]
#[case::empty("", CompresrTransportError::Scheme)]
#[case::empty_host("http://", CompresrTransportError::NoHost)]
#[case::bad_port("http://api.compresr.ai:99999", CompresrTransportError::InvalidApiBase)]
fn api_base_rejects_bad_schemes_and_hosts(
    #[case] url: &str,
    #[case] expected: CompresrTransportError,
) {
    assert_eq!(validate_api_base(url), Err(expected));
}
