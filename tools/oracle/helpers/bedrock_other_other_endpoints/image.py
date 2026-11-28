from typing import Literal

import litellm
from pydantic import JsonValue

import bedrock_image_dispatch

from .common import (
    JSON,
    failure_row,
    params_of,
    probe_images,
    reply_body,
    reply_headers,
    reply_status,
    request_row,
    success_row,
)
from .models import (
    IMAGE_INPUT,
    IMAGE_OUTPUT,
    ImageRow,
    Probe,
)


def image_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    return {
        "status": reply_status(reply),
        "headers": reply_headers(reply),
        "body": reply_body(reply),
    }


def image_probe(probe: Probe) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {"model": probe["model"], "prompt": probe.get("prompt")}
    raw["images"] = JSON.validate_python(probe_images(probe))
    raw["extra_headers"] = JSON.validate_python(probe.get("extra_headers"))
    raw.update({"optional_params": params_of(probe), "api_base": probe.get("api_base")})
    raw["api_key"], raw["reply"] = probe.get("api_key"), image_reply(probe)
    return raw


def image_value(row: ImageRow) -> JsonValue:
    raw: dict[str, JsonValue] = {"images": row["images"], "cost": row["cost"]}
    return JSON.validate_python(raw)


def image_row(
    model_cost: dict[str, JsonValue] | None,
    probe: Probe,
    case: Literal["canvas_image", "canvas_edit"],
) -> JsonValue:
    cost_map: dict[str, JsonValue] = {}
    if model_cost is not None:
        cost_map = model_cost
    litellm.model_cost = cost_map
    probes: list[JsonValue] = [image_probe(probe)]
    raw: dict[str, JsonValue] = {"case": case, "model_cost": cost_map, "probes": probes}
    output = IMAGE_OUTPUT.validate_python(
        bedrock_image_dispatch.run(IMAGE_INPUT.validate_python(raw))
    )
    row = output[0]
    requests: list[JsonValue] = []
    request = row["request"]
    url = request["url"]
    if url is not None:
        requests.append(request_row(url, request["body"], request["authorization"]))
    if row["ok"]:
        return success_row(requests, image_value(row))
    return failure_row(requests, row["status"], None)
