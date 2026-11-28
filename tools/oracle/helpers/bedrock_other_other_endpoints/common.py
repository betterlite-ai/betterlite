from typing import Final

from litellm.types.llms.bedrock import AwsAuthParams
from pydantic import JsonValue, TypeAdapter

from .models import Probe, Reply

NOW: Final = 1_700_000_000
UNSUPPORTED_FORMAT: Final = "unsupported_format"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def params_of(probe: Probe) -> dict[str, JsonValue]:
    params = probe.get("optional_params")
    if params is None:
        return {}
    return params


def environment_of(probe: Probe) -> dict[str, str]:
    environment = probe.get("environment")
    if environment is None:
        return {}
    return environment


def input_values(probe: Probe) -> list[str]:
    values = probe.get("input")
    if values is None:
        return []
    return values


def query_value(probe: Probe) -> str:
    query = probe.get("query")
    if query is None:
        return ""
    return query


def request_model_of(probe: Probe) -> str:
    request_model = probe.get("request_model")
    if request_model is None:
        return probe["model"]
    return request_model


def probe_images(probe: Probe) -> list[str]:
    images = probe.get("images")
    if images is None:
        return []
    return images


def documents_of(probe: Probe) -> list[JsonValue]:
    documents = probe.get("documents")
    if documents is None:
        return []
    return documents


def replies_of(probe: Probe) -> list[Reply]:
    replies = probe.get("replies")
    if replies is not None:
        return replies
    reply = probe.get("reply")
    if reply is None:
        empty: Reply = {}
        return [empty]
    return [reply]


def reply_status(reply: Reply | None) -> int:
    if reply is None:
        return 200
    status = reply.get("status")
    if status is None:
        return 200
    return status


def reply_body(reply: Reply | None) -> str:
    if reply is None:
        return "{}"
    body = reply.get("body")
    if body is None:
        return "{}"
    return body


def reply_headers(reply: Reply | None) -> JsonValue:
    headers: dict[str, str] = {}
    if reply is not None:
        found = reply.get("headers")
        if found is not None:
            headers = found
    return JSON.validate_python(headers)


def region_of(probe: Probe) -> str:
    region = probe.get("region")
    if region is None:
        return "us-east-1"
    return region


def keys_of(probe: Probe) -> dict[str, JsonValue]:
    params = AwsAuthParams.model_validate(params_of(probe))
    return {
        "access_key_id": params.aws_access_key_id,
        "secret_access_key": params.aws_secret_access_key,
    }


def credentials_of(probe: Probe) -> dict[str, JsonValue]:
    keys = keys_of(probe)
    keys["session_token"] = None
    return keys


def header_marker(value: str | None) -> str | None:
    if value is None:
        return None
    if value.startswith("AWS4-HMAC-SHA256"):
        return "sigv4"
    return "other"


def status_header(headers: list[list[str]]) -> str | None:
    for pair in headers:
        if pair[0].lower() == "authorization":
            return header_marker(pair[1])
    return None


def parsed_body(text: str) -> JsonValue:
    if text == "":
        return None
    return JSON.validate_json(text)


def request_row(url: str, body: JsonValue, authorization: str | None) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "url": url,
        "body": body,
        "authorization": authorization,
    }
    return JSON.validate_python(raw)


def success_row(requests: list[JsonValue], value: JsonValue) -> JsonValue:
    raw: dict[str, JsonValue] = {"requests": requests, "ok": True, "status": None, "error": None}
    raw["value"] = value
    return raw


def failure_row(requests: list[JsonValue], status: int | None, error: str | None) -> JsonValue:
    raw: dict[str, JsonValue] = {"requests": requests, "ok": False, "status": status}
    raw["error"] = error
    raw["value"] = None
    return raw
