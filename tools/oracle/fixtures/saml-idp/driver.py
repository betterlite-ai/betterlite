import base64
import pathlib
import sys
from typing import Literal
from xml.sax.saxutils import escape, quoteattr

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from onelogin.saml2.utils import OneLogin_Saml2_Utils
from pydantic import BaseModel

MAX_INPUT_BYTES = 64 * 1024
WRONG_AUDIENCE = "urn:betterlite:oracle:wrong-audience"
WRONG_DESTINATION = "https://wrong-destination.invalid/saml/acs"
RESPONSE_ID_PREFIX = "_response_"
ASSERTION_NAMESPACE = "urn:oasis:names:tc:SAML:2.0:assertion"
PROTOCOL_NAMESPACE = "urn:oasis:names:tc:SAML:2.0:protocol"
NAMEID_FORMAT = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"
BEARER_CONFIRMATION = "urn:oasis:names:tc:SAML:2.0:cm:bearer"
SUCCESS_STATUS = "urn:oasis:names:tc:SAML:2.0:status:Success"
XML_DECLARATION = '<?xml version="1.0"?>'

Mutation = Literal[
    "unsigned",
    "wrong_key",
    "expired",
    "wrong_audience",
    "wrong_destination",
    "subject_confirmation_only",
]


class SigningInput(BaseModel):
    key_directory: str
    issuer: str
    subject: str
    attributes: dict[str, list[str]]
    audience: str
    destination: str
    in_response_to: str
    assertion_id: str
    issue_instant: str
    not_before: str
    not_on_or_after: str
    mutation: Mutation | None = None


def signed_response(document: SigningInput) -> str:
    scd_only = document.mutation == "subject_confirmation_only"
    audience = WRONG_AUDIENCE if document.mutation == "wrong_audience" else document.audience
    destination = (
        WRONG_DESTINATION if document.mutation == "wrong_destination" else document.destination
    )
    in_response_to = f" InResponseTo={quoteattr(document.in_response_to)}"
    response_in_response_to = "" if scd_only else in_response_to
    attributes = "".join(
        f"<saml:Attribute Name={quoteattr(name)}>"
        + "".join(
            f"<saml:AttributeValue>{escape(value)}</saml:AttributeValue>"
            for value in values
        )
        + "</saml:Attribute>"
        for name, values in document.attributes.items()
    )
    assertion = (
        f"<saml:Assertion xmlns:saml={quoteattr(ASSERTION_NAMESPACE)} "
        f"ID={quoteattr(document.assertion_id)} "
        'Version="2.0" '
        f"IssueInstant={quoteattr(document.issue_instant)}>"
        f"<saml:Issuer>{escape(document.issuer)}</saml:Issuer>"
        "<saml:Subject>"
        f"<saml:NameID Format={quoteattr(NAMEID_FORMAT)}>{escape(document.subject)}</saml:NameID>"
        f"<saml:SubjectConfirmation Method={quoteattr(BEARER_CONFIRMATION)}>"
        f"<saml:SubjectConfirmationData{in_response_to} "
        f"NotOnOrAfter={quoteattr(document.not_on_or_after)} "
        f"Recipient={quoteattr(destination)}/>"
        "</saml:SubjectConfirmation></saml:Subject>"
        f"<saml:Conditions NotBefore={quoteattr(document.not_before)} "
        f"NotOnOrAfter={quoteattr(document.not_on_or_after)}>"
        f"<saml:AudienceRestriction><saml:Audience>{escape(audience)}</saml:Audience>"
        "</saml:AudienceRestriction></saml:Conditions>"
        f"<saml:AuthnStatement AuthnInstant={quoteattr(document.issue_instant)} "
        'SessionIndex="_session">'
        "<saml:AuthnContext><saml:AuthnContextClassRef>"
        "urn:oasis:names:tc:SAML:2.0:ac:classes:Password"
        "</saml:AuthnContextClassRef></saml:AuthnContext></saml:AuthnStatement>"
        f"<saml:AttributeStatement>{attributes}</saml:AttributeStatement>"
        "</saml:Assertion>"
    )
    if document.mutation != "unsigned":
        directory = pathlib.Path(document.key_directory)
        key_pem = (directory / "key.pem").read_text(encoding="ascii")
        cert_pem = (directory / "cert.pem").read_text(encoding="ascii")
        if document.mutation == "wrong_key":
            rogue = rsa.generate_private_key(public_exponent=65537, key_size=2048)
            key_pem = rogue.private_bytes(
                serialization.Encoding.PEM,
                serialization.PrivateFormat.TraditionalOpenSSL,
                serialization.NoEncryption(),
            ).decode("ascii")
        signed = OneLogin_Saml2_Utils.add_sign(assertion, key_pem, cert_pem)
        text = signed.decode("utf-8") if isinstance(signed, bytes) else signed
        assertion = text.replace(XML_DECLARATION, "")
    response_id = f"{RESPONSE_ID_PREFIX}{OneLogin_Saml2_Utils.generate_unique_id()}"
    response = (
        XML_DECLARATION
        + f"<samlp:Response xmlns:samlp={quoteattr(PROTOCOL_NAMESPACE)} "
        f"xmlns:saml={quoteattr(ASSERTION_NAMESPACE)} "
        f"ID={quoteattr(response_id)} "
        'Version="2.0" '
        f"IssueInstant={quoteattr(document.issue_instant)} "
        f"Destination={quoteattr(destination)}{response_in_response_to}>"
        f"<saml:Issuer>{escape(document.issuer)}</saml:Issuer>"
        f'<samlp:Status><samlp:StatusCode Value={quoteattr(SUCCESS_STATUS)}/>'
        "</samlp:Status>"
        f"{assertion}</samlp:Response>"
    )
    return base64.b64encode(response.encode("utf-8")).decode("ascii")


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise SystemExit("the signing input is too large")
    document = SigningInput.model_validate_json(raw)
    _ = sys.stdout.write(signed_response(document))


main()
