from litellm.proxy._experimental.mcp_server.outbound_credentials.types import CredError
from pydantic import JsonValue


def error_from_tag(tag: str, detail: str) -> CredError:
    match tag:
        case "misconfigured":
            error = CredError.of_misconfigured(detail)
        case "upstream_unavailable":
            error = CredError.of_upstream_unavailable(detail)
        case "precondition_required":
            error = CredError.of_precondition_required(detail)
        case "not_implemented":
            error = CredError.of_not_implemented(detail)
        case "unsupported_mode":
            error = CredError.of_unsupported_mode(detail)
        case "url_credentials_not_allowed":
            error = CredError.of_url_credentials_not_allowed()
        case _:
            error = CredError.of_misconfigured(detail)
    return error


def error_record(action: str, error: CredError) -> dict[str, JsonValue]:
    www_authenticate: str | None = None
    if error.tag == "unauthorized":
        www_authenticate = error.unauthorized.www_authenticate
    return {
        "action": action,
        "ok": False,
        "tag": error.tag,
        "summary": error.summary,
        "www_authenticate": www_authenticate,
    }
