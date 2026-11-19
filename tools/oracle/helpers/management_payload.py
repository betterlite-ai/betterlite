from collections.abc import MutableMapping
from typing import Final, Literal, NotRequired, TypedDict, cast

from litellm.proxy.management_helpers.utils import (
    _redact_env_var_values,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/utils.py#L650
    _redact_record_env_vars,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/utils.py#L629
)
from litellm.types.mcp import MCPTransport, MCPTransportType
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter

CASE_SERVER_NAME: Final = "case-server"
TRANSPORTS: Final[dict[str, MCPTransportType]] = {
    "sse": MCPTransport.sse,
    "http": MCPTransport.http,
    "stdio": MCPTransport.stdio,
}


class RedactEnvVarsCall(TypedDict):
    function: Literal["redact_env_vars"]
    response: dict[str, JsonValue]


class RedactMappingCall(TypedDict):
    function: Literal["redact_mapping"]
    record: dict[str, JsonValue]


class RedactOtherCall(TypedDict):
    function: Literal["redact_other"]
    record: JsonValue


class RedactModelCall(TypedDict):
    function: Literal["redact_model"]
    server_id: str
    transport: Literal["sse", "http", "stdio"]
    env_vars: NotRequired[list[dict[str, JsonValue]] | None]


class RedactedModel(TypedDict):
    env_vars: list[dict[str, JsonValue]] | None


class Input(TypedDict):
    function: str


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
REDACT_ENV_VARS: TypeAdapter[RedactEnvVarsCall] = TypeAdapter(RedactEnvVarsCall)
REDACT_MAPPING: TypeAdapter[RedactMappingCall] = TypeAdapter(RedactMappingCall)
REDACT_OTHER: TypeAdapter[RedactOtherCall] = TypeAdapter(RedactOtherCall)
REDACT_MODEL: TypeAdapter[RedactModelCall] = TypeAdapter(RedactModelCall)
REDACTED_MODEL: TypeAdapter[RedactedModel] = TypeAdapter(RedactedModel)


def env_vars_answer(call: RedactEnvVarsCall) -> JsonValue:
    result = call["response"]
    response = dict(result)
    _redact_env_var_values(cast(MutableMapping[str, object], response))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/utils.py#L650
    answer: dict[str, JsonValue] = {"response": response, "result": result}
    return JSON.validate_python(answer)


def model_answer(call: RedactModelCall) -> JsonValue:
    server = MCPServer(
        server_id=call["server_id"],
        name=CASE_SERVER_NAME,
        transport=TRANSPORTS[call["transport"]],
        env_vars=call.get("env_vars"),
    )
    redacted = cast(MCPServer, _redact_record_env_vars(server))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/utils.py#L629
    projected = REDACTED_MODEL.validate_json(redacted.model_dump_json())
    return {"env_vars": JSON.validate_python(projected["env_vars"])}


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "redact_env_vars":
        return env_vars_answer(REDACT_ENV_VARS.validate_python(input_value))
    if function == "redact_mapping":
        mapping = REDACT_MAPPING.validate_python(input_value)
        return {"record": JSON.validate_python(_redact_record_env_vars(mapping["record"]))}
    if function == "redact_model":
        return model_answer(REDACT_MODEL.validate_python(input_value))
    if function == "redact_other":
        other = REDACT_OTHER.validate_python(input_value)
        return {"record": JSON.validate_python(_redact_record_env_vars(other["record"]))}
    raise ValueError(f"unknown function {function}")
