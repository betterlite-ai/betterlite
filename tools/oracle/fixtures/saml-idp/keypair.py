import datetime
import pathlib
import sys
from xml.sax.saxutils import quoteattr

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID
from pydantic import BaseModel

MAX_INPUT_BYTES = 64 * 1024
COMMON_NAME = "betterlite-oracle-idp"
IDP_METADATA_NAMESPACE = "urn:oasis:names:tc:SAML:2.0:metadata"
SAML_PROTOCOL_NAMESPACE = "urn:oasis:names:tc:SAML:2.0:protocol"
XMLDSIG_NAMESPACE = "http://www.w3.org/2000/09/xmldsig#"
REDIRECT_BINDING = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect"


class KeypairInput(BaseModel):
    entity_id: str
    sso_url: str
    output_directory: str


def metadata_xml(document: KeypairInput, cert_pem: str) -> str:
    cert_body = "".join(
        line for line in cert_pem.splitlines() if "CERTIFICATE" not in line
    )
    return (
        '<?xml version="1.0"?>'
        f"<EntityDescriptor xmlns={quoteattr(IDP_METADATA_NAMESPACE)} "
        f"entityID={quoteattr(document.entity_id)}>"
        f"<IDPSSODescriptor protocolSupportEnumeration={quoteattr(SAML_PROTOCOL_NAMESPACE)}>"
        f'<KeyDescriptor use="signing"><KeyInfo xmlns={quoteattr(XMLDSIG_NAMESPACE)}>'
        f"<X509Data><X509Certificate>{cert_body}</X509Certificate></X509Data>"
        "</KeyInfo></KeyDescriptor>"
        f"<SingleSignOnService Binding={quoteattr(REDIRECT_BINDING)} "
        f"Location={quoteattr(document.sso_url)}/>"
        "</IDPSSODescriptor></EntityDescriptor>"
    )


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise SystemExit("the keypair input is too large")
    document = KeypairInput.model_validate_json(raw)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, COMMON_NAME)])
    now = datetime.datetime.now(datetime.UTC)
    certificate = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=365))
        .sign(key, hashes.SHA256())
    )
    key_pem = key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL,
        serialization.NoEncryption(),
    ).decode("ascii")
    cert_pem = certificate.public_bytes(serialization.Encoding.PEM).decode("ascii")
    directory = pathlib.Path(document.output_directory)
    directory.mkdir(parents=True, exist_ok=True)
    _ = (directory / "key.pem").write_text(key_pem, encoding="ascii")
    _ = (directory / "cert.pem").write_text(cert_pem, encoding="ascii")
    _ = (directory / "metadata.xml").write_text(
        metadata_xml(document, cert_pem), encoding="utf-8"
    )


main()
