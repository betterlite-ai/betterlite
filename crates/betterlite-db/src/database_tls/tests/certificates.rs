use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub(super) struct Authority {
    pub(super) certificate_pem: String,
    pub(super) leaf_pem: String,
    pub(super) key_pem: String,
    pub(super) leaf: CertificateDer<'static>,
    pub(super) key: PrivateKeyDer<'static>,
}

pub(super) fn authority() -> Authority {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca_certificate = ca_params.self_signed(&ca_key).expect("a CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let key = KeyPair::generate().expect("a leaf key");
    // rustls matches the DNS subject alternative name and ignores the common name
    let mut leaf_params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("leaf parameters");
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_certificate = leaf_params
        .signed_by(&key, &issuer)
        .expect("a leaf certificate");
    Authority {
        certificate_pem: ca_certificate.pem(),
        leaf_pem: leaf_certificate.pem(),
        key_pem: key.serialize_pem(),
        leaf: leaf_certificate.der().clone(),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    }
}

pub(super) fn unrelated_ca_pem() -> String {
    let key = KeyPair::generate().expect("a CA key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.self_signed(&key).expect("a CA certificate").pem()
}

pub(super) fn self_signed() -> Authority {
    let key = KeyPair::generate().expect("a leaf key");
    let mut params = CertificateParams::new(vec!["localhost".to_owned()]).expect("leaf parameters");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = params.self_signed(&key).expect("a self-signed certificate");
    Authority {
        certificate_pem: certificate.pem(),
        leaf_pem: certificate.pem(),
        key_pem: key.serialize_pem(),
        leaf: certificate.der().clone(),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    }
}
