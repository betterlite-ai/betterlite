import json
import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.llms.bedrock.count_tokens.transformation import BedrockCountTokensConfig
from pydantic import BaseModel, ConfigDict, JsonValue

JsonMap: TypeAlias = dict[str, JsonValue]


class SingleCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    request: JsonMap = {}


class MultiCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    requests: list[JsonMap] = []


class EndpointCase(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str
    aws_region_name: str
    api_base: str | None = None
    aws_bedrock_runtime_endpoint: str | None = None
    environment: dict[str, str] = {}


class EndpointCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    cases: list[EndpointCase] = []


class ResponseCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    cases: list[JsonMap] = []


def to_json(value: object) -> JsonValue:
    return cast("JsonValue", json.loads(json.dumps(value)))


def single_request(argument: JsonValue) -> JsonValue:
    call = SingleCall.model_validate(argument)
    config = BedrockCountTokensConfig()
    config.validate_count_tokens_request(call.request)
    return to_json(config.transform_anthropic_to_bedrock_count_tokens(call.request))


def outcome(request: JsonMap) -> JsonValue:
    config = BedrockCountTokensConfig()
    input_type = config._detect_input_type(request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        config.validate_count_tokens_request(request)
    except ValueError:
        return to_json({"input_type": input_type, "validated": False})
    return to_json(
        {
            "input_type": input_type,
            "validated": True,
            "request": config.transform_anthropic_to_bedrock_count_tokens(request),
        }
    )


def multi_request(argument: JsonValue) -> JsonValue:
    call = MultiCall.model_validate(argument)
    return to_json({"outcomes": [outcome(request) for request in call.requests]})


def endpoint(argument: JsonValue) -> JsonValue:
    call = EndpointCall.model_validate(argument)
    config = BedrockCountTokensConfig()
    endpoints: list[JsonValue] = []
    for case in call.cases:
        with patch.dict(os.environ, case.environment, clear=True):
            endpoints.append(
                config.get_bedrock_count_tokens_endpoint(
                    model=case.model,
                    aws_region_name=case.aws_region_name,
                    api_base=case.api_base,
                    aws_bedrock_runtime_endpoint=case.aws_bedrock_runtime_endpoint,
                )
            )
    return to_json({"endpoints": endpoints})


def response(argument: JsonValue) -> JsonValue:
    call = ResponseCall.model_validate(argument)
    config = BedrockCountTokensConfig()
    return to_json(
        {"results": [config.transform_bedrock_response_to_anthropic(case) for case in call.cases]}
    )


HANDLERS: dict[str, Callable[[JsonValue], JsonValue]] = {
    "typed_block_invoke_model": single_request,
    "untyped_messages_converse": multi_request,
    "tool_name_schema": single_request,
    "count_endpoint_path": endpoint,
    "count_to_anthropic_result": response,
    "invoke_body_python_json_spacing": single_request,
    "invoke_body_defaults": multi_request,
    "converse_content_system": multi_request,
    "count_model_missing": single_request,
    "count_messages_empty": single_request,
    "count_message_not_object": single_request,
    "count_message_role_missing": single_request,
    "count_message_content_missing": single_request,
    "count_invoke_model_only": single_request,
    "empty_tools_omission": multi_request,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
