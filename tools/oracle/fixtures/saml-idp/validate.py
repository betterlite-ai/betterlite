import sys
from urllib.parse import urlsplit

from onelogin.saml2.auth import OneLogin_Saml2_Auth
from onelogin.saml2.idp_metadata_parser import OneLogin_Saml2_IdPMetadataParser
from pydantic import BaseModel

MAX_INPUT_BYTES = 1024 * 1024
POST_BINDING = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
NAMEID_FORMAT = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"


class ValidationInput(BaseModel):
    saml_response: str
    request_id: str
    audience: str
    destination: str
    metadata: str


class ValidationOutput(BaseModel):
    valid: bool
    error: str
    response_in_response_to: str
    assertion_id: str


def validate(document: ValidationInput) -> ValidationOutput:
    parsed = OneLogin_Saml2_IdPMetadataParser.parse(document.metadata)
    settings: dict[str, object] = {
        "strict": True,
        "debug": False,
        "sp": {
            "entityId": document.audience,
            "assertionConsumerService": {
                "url": document.destination,
                "binding": POST_BINDING,
            },
            "NameIDFormat": NAMEID_FORMAT,
        },
        "security": {
            "wantAssertionsSigned": True,
            "wantMessagesSigned": False,
            "authnRequestsSigned": False,
            "wantNameId": True,
            "requestedAuthnContext": False,
            "rejectUnsolicitedResponsesWithInResponseTo": False,
        },
    }
    merged = OneLogin_Saml2_IdPMetadataParser.merge_settings(settings, parsed)
    split = urlsplit(document.destination)
    empty_query: dict[str, str] = {}
    request_data: dict[str, object] = {
        "https": "on" if split.scheme == "https" else "off",
        "http_host": split.netloc,
        "script_name": split.path,
        "get_data": empty_query,
        "post_data": {"SAMLResponse": document.saml_response},
    }
    auth = OneLogin_Saml2_Auth(request_data, old_settings=merged)
    _ = auth.process_response(request_id=document.request_id)
    reason = auth.get_last_error_reason()
    return ValidationOutput(
        valid=auth.is_authenticated(),
        error=reason if reason is not None else " ".join(auth.get_errors()),
        response_in_response_to=auth.get_last_response_in_response_to() or "",
        assertion_id=auth.get_last_assertion_id() or "",
    )


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise SystemExit("the validation input is too large")
    document = ValidationInput.model_validate_json(raw)
    _ = sys.stdout.write(validate(document).model_dump_json())


main()
