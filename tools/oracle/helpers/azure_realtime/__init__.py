import asyncio

import litellm
from pydantic import JsonValue

from .http import http_answer
from .probe import connect_one, failure_probe, protocol_one
from .types import JSON, Input


def connect_answer(input_value: Input) -> JsonValue:
    connects = [
        asyncio.run(connect_one(input_value, setup)) for setup in input_value["queries"]
    ]
    payload: dict[str, JsonValue] = {"connects": connects}
    return JSON.validate_python(payload)


def protocol_answer(input_value: Input) -> JsonValue:
    protocols = [protocol_one(input_value, setup) for setup in input_value["protocols"]]
    payload: dict[str, JsonValue] = {"protocols": JSON.validate_python(protocols)}
    return JSON.validate_python(payload)


def failure_answer(input_value: Input) -> JsonValue:
    failures = [
        asyncio.run(failure_probe(input_value, failure))
        for failure in input_value["failures"]
    ]
    payload: dict[str, JsonValue] = {"failures": failures}
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    kind = input_value["kind"]
    if kind == "http":
        return http_answer(input_value)
    if kind == "protocol":
        return protocol_answer(input_value)
    if kind == "connect":
        return connect_answer(input_value)
    return failure_answer(input_value)
