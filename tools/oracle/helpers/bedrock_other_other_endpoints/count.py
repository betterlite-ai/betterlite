from typing import Final

from pydantic import JsonValue

import bedrock_other_count_call

from .common import (
    JSON,
    NOW,
    environment_of,
    header_marker,
    params_of,
    parsed_body,
    reply_body,
    reply_headers,
    reply_status,
    request_model_of,
    request_row,
    success_row,
)
from .models import (
    COUNT_INPUT,
    COUNT_OUTPUT,
    CountRequest,
    CountResult,
    Probe,
)

COUNT_ENV_NAMES: Final = (
    ("region_name", "AWS_REGION_NAME"),
    ("region", "AWS_REGION"),
    ("session_region", "AWS_DEFAULT_REGION"),
    ("runtime_endpoint", "AWS_BEDROCK_RUNTIME_ENDPOINT"),
    ("bearer_token", "AWS_BEARER_TOKEN_BEDROCK"),
)


def count_environment(probe: Probe) -> dict[str, JsonValue]:
    environment = environment_of(probe)
    raw: dict[str, JsonValue] = {}
    for name, env in COUNT_ENV_NAMES:
        raw[name] = environment.get(env)
    return raw


def count_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    failure = reply is not None and reply.get("failure") is True
    return {
        "status": reply_status(reply),
        "body": reply_body(reply),
        "headers": reply_headers(reply),
        "failure": failure,
    }


def count_call(probe: Probe) -> JsonValue:
    deployment: dict[str, JsonValue] = {"litellm_params": params_of(probe)}
    return {
        "model": probe["model"], "now": NOW, "request_model": request_model_of(probe),
        "messages": JSON.validate_python(probe.get("messages")), "deployment": deployment,
        "environment": count_environment(probe), "reply": count_reply(probe),
    }


def count_value(result: CountResult | None) -> JsonValue:
    if result is None:
        return None
    raw: dict[str, JsonValue] = {
        "total_tokens": result["total_tokens"],
        "request_model": result["request_model"],
        "model_used": result["model_used"],
        "tokenizer_type": result["tokenizer_type"],
        "error": result["error"],
        "error_message": result["error_message"],
        "status_code": result["status_code"],
        "original_response": result["original_response"],
    }
    return JSON.validate_python(raw)


def count_request(request: CountRequest) -> JsonValue:
    authorization = header_marker(request["headers"].get("authorization"))
    return request_row(request["url"], parsed_body(request["body"]), authorization)


def count_row(probe: Probe) -> JsonValue:
    calls: list[JsonValue] = [count_call(probe)]
    raw: dict[str, JsonValue] = {"count_token_http_request": {"calls": calls}}
    output = COUNT_OUTPUT.validate_python(
        bedrock_other_count_call.run(COUNT_INPUT.validate_python(raw))
    )
    call = output["calls"][0]
    requests: list[JsonValue] = []
    request = call["request"]
    if request is not None:
        requests.append(count_request(request))
    return success_row(requests, count_value(call["result"]))
