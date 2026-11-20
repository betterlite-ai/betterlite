import asyncio

import litellm
from pydantic import JsonValue

from .operations import (
    agent_answer,
    agent_resolution_answer,
    mcp_per_key_answer,
    mcp_per_team_answer,
    session_answer,
    tag_create_answer,
    tag_descriptor_answer,
    tag_per_key_answer,
    tag_resolution_answer,
)
from .types import JSON, Input, LimiterCall

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "tag_resolution":
        return JSON.validate_python(asyncio.run(tag_resolution_answer(input_value)))
    if input_value["function"] == "agent_resolution":
        return JSON.validate_python(agent_resolution_answer(input_value))
    if input_value["function"] == "session_resolution":
        return JSON.validate_python(session_answer(input_value))
    return limiter_answer(input_value)


def limiter_answer(input_value: LimiterCall) -> JsonValue:
    if input_value["function"] == "tag_descriptor":
        return JSON.validate_python(tag_descriptor_answer(input_value))
    if input_value["function"] == "tag_create":
        return JSON.validate_python(tag_create_answer(input_value))
    if input_value["function"] == "tag_per_key":
        return JSON.validate_python(tag_per_key_answer(input_value))
    if input_value["function"] == "mcp_per_key":
        return JSON.validate_python(mcp_per_key_answer(input_value))
    if input_value["function"] == "mcp_per_team":
        return JSON.validate_python(mcp_per_team_answer(input_value))
    return JSON.validate_python(agent_answer(input_value))
