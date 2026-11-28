from typing import Literal

from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue

import bedrock_other_embedding_call
import bedrock_other_embedding_jobs

from .common import (
    JSON,
    NOW,
    environment_of,
    failure_row,
    header_marker,
    input_values,
    keys_of,
    params_of,
    parsed_body,
    region_of,
    replies_of,
    reply_body,
    reply_headers,
    reply_status,
    request_row,
    status_header,
    success_row,
)
from .models import (
    EMBEDDING_OUTPUT,
    EMBEDDING_PAYLOAD,
    JOBS_INPUT,
    STATUS_OUTPUT,
    EmbeddingRequest,
    Probe,
    Reply,
    StatusOutput,
    StatusRequest,
)


def embedding_reply(reply: Reply) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {"status": reply_status(reply), "body": reply_body(reply)}
    raw["headers"] = reply_headers(reply)
    if reply.get("failure") is True:
        raw["failure"] = "failed"
    return raw


def supplied_kind(probe: Probe) -> Literal["none", "sync"]:
    # cohere uses the legacy transport; the batch families ask the factory for a client
    if probe["model"].startswith("cohere."):
        return "sync"
    return "none"


def embedding_case(probe: Probe) -> dict[str, JsonValue]:
    replies: list[JsonValue] = [
        JSON.validate_python(embedding_reply(reply)) for reply in replies_of(probe)
    ]
    raw: dict[str, JsonValue] = {
        "function": "embeddings",
        "model": probe["model"],
        "input": JSON.validate_python(input_values(probe)),
        "now": NOW,
        "optional_params": params_of(probe),
        "api_base": probe.get("api_base"),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "api_key": probe.get("api_key"),
        "timeout": probe.get("timeout"),
        "aembedding": False,
        "drop_params": False,
        "supplied": supplied_kind(probe),
        "environment": JSON.validate_python(environment_of(probe)),
        "replies": JSON.validate_python(replies),
    }
    return raw


def embedding_value(
    response: EmbeddingResponse,
    factory: list[JsonValue],
    phases: list[JsonValue],
) -> JsonValue:
    payload = EMBEDDING_PAYLOAD.validate_json(response.model_dump_json())
    data: list[JsonValue] = [
        {"index": record["index"], "embedding": record["embedding"]}
        for record in payload["data"]
    ]
    usage: dict[str, JsonValue] = {
        "prompt_tokens": payload["usage"]["prompt_tokens"],
        "total_tokens": payload["usage"]["total_tokens"],
    }
    raw: dict[str, JsonValue] = {
        "model": payload["model"],
        "data": data,
        "usage": usage,
        "factory": JSON.validate_python(factory),
        "phases": JSON.validate_python(phases),
    }
    return JSON.validate_python(raw)


def embedding_request(request: EmbeddingRequest) -> JsonValue:
    authorization = header_marker(request["headers"].get("authorization"))
    return request_row(request["url"], request["body"], authorization)


def embedding_row(probe: Probe) -> JsonValue:
    output = EMBEDDING_OUTPUT.validate_python(
        bedrock_other_embedding_call.run(embedding_case(probe))
    )
    requests: list[JsonValue] = [
        embedding_request(request) for request in output["requests"]
    ]
    if not output["ok"]:
        return failure_row(requests, output["status"], None)
    value: JsonValue = None
    response = output["response"]
    if response is not None:
        value = embedding_value(response, output["factory"], output["phases"])
    return success_row(requests, value)


def status_request(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    raw: dict[str, JsonValue] = {
        "function": "status",
        "invocation_arn": probe.get("invocation_arn"),
        "region": region_of(probe),
        "environment": JSON.validate_python(environment_of(probe)),
        "credentials": JSON.validate_python(keys_of(probe)),
        "status": reply_status(reply),
        "body": reply_body(reply),
    }
    logging = probe.get("logging")
    if logging is not None:
        raw["logging"] = logging
    endpoint = params_of(probe).get("aws_bedrock_runtime_endpoint")
    if endpoint is not None:
        raw["endpoint"] = endpoint
    return raw


def status_phases(output: StatusOutput) -> list[JsonValue]:
    phases: list[JsonValue] = []
    if output["pre"] is not None:
        phases.append("pre_api_call")
    if output["post"] is not None:
        phases.append("post_api_call")
    return phases


def status_value(output: StatusOutput) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "body": output["value"],
        "regions": JSON.validate_python(output["resolved_regions"]),
        "phases": status_phases(output),
    }
    return JSON.validate_python(raw)


def status_request_row(request: StatusRequest) -> JsonValue:
    body = parsed_body(request["body"])
    return request_row(request["url"], body, status_header(request["headers"]))


def status_row(probe: Probe) -> JsonValue:
    output = STATUS_OUTPUT.validate_python(
        bedrock_other_embedding_jobs.run(JOBS_INPUT.validate_python(status_request(probe)))
    )
    requests: list[JsonValue] = [
        status_request_row(request) for request in output["requests"]
    ]
    if output["kind"] == "returned":
        return success_row(requests, status_value(output))
    return failure_row(requests, None, None)
