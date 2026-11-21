#[derive(Clone, Copy, Debug, Default)]
pub struct RequestProvenance<'request> {
    pub forwarded_for: Option<&'request str>,
    pub client_host: Option<&'request str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpVerdict<'request> {
    pub allowed: bool,
    pub passed_in_ip: Option<&'request str>,
}

#[must_use]
pub fn request_ip_address<'request>(
    provenance: &RequestProvenance<'request>,
    use_x_forwarded_for: bool,
) -> &'request str {
    if use_x_forwarded_for && let Some(value) = provenance.forwarded_for {
        return value;
    }
    provenance.client_host.unwrap_or_default()
}

#[must_use]
pub fn check_valid_ip<'request>(
    allowed_ips: Option<&[String]>,
    provenance: &RequestProvenance<'request>,
    use_x_forwarded_for: bool,
) -> IpVerdict<'request> {
    let Some(allowed_ips) = allowed_ips else {
        return IpVerdict {
            allowed: true,
            passed_in_ip: None,
        };
    };
    let client_ip = request_ip_address(provenance, use_x_forwarded_for);
    let allowed = allowed_ips
        .iter()
        .any(|candidate| candidate.as_str() == client_ip);
    IpVerdict {
        allowed,
        passed_in_ip: Some(client_ip),
    }
}
