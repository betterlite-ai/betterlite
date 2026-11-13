from collections.abc import Callable, Mapping
from typing import Final, TypeAlias, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.base_llm.passthrough.transformation import (
    RELAYED_JSON_OBJECT,
    PassthroughMetadata,
    RelayShape,
    logged_relay_shape,
    model_group_from,
    relayed_body,  # pyright: ignore[reportUnknownVariableType]
    relayed_json_object,
    replace_path_segment,
    strip_leading_model_segment,
)
from litellm.types.llms.openai import ResponsesAPIResponse, ResponsesTerminalEvent
from litellm.types.rerank import RerankResponse
from litellm.types.utils import CallTypes, CostResponseTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
LoggedRelayResponse: TypeAlias = (
    CostResponseTypes | RerankResponse | ResponsesAPIResponse | ResponsesTerminalEvent
)
DEFAULT_STATUS: Final = 200


class Logging:
    def __init__(self) -> None:
        self.call_type: str | None = None


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(Object, value)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def status_value(fields: Object) -> int:
    value = fields.get("status_code", DEFAULT_STATUS)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("status_code must be an integer")
    return value


def relayed_response(fields: Object) -> httpx.Response:
    body = string_value(fields.get("body"), "body")
    return httpx.Response(status_value(fields), content=body.encode())


def mapping_parse(body: Mapping[str, object]) -> LoggedRelayResponse:
    validated = RELAYED_JSON_OBJECT.validate_python(body)
    return cast(LoggedRelayResponse, dict(validated))


def failing_parse(_body: Mapping[str, object]) -> LoggedRelayResponse:
    return cast(LoggedRelayResponse, PassthroughMetadata.model_validate(None))


CALL_TYPES: Final[dict[str, CallTypes]] = {
    "aembedding": CallTypes.aembedding,
    "aimage_generation": CallTypes.aimage_generation,
    "arerank": CallTypes.arerank,
    "aresponses": CallTypes.aresponses,
}
PARSE: Final[dict[str, Callable[[Mapping[str, object]], LoggedRelayResponse]]] = {
    "mapping": mapping_parse,
    "fail": failing_parse,
}


def call_type(name: str) -> CallTypes:
    value = CALL_TYPES.get(name)
    if value is None:
        raise ValueError(f"unknown call type {name}")
    return value


def shape(value: Json) -> RelayShape:
    fields = object_value(value, "shape")
    parse_name = string_value(fields.get("parse"), "parse")
    parse = PARSE.get(parse_name)
    if parse is None:
        raise ValueError(f"unknown parse {parse_name}")
    return RelayShape(
        path_suffix=string_value(fields.get("path_suffix"), "path_suffix"),
        call_type=call_type(string_value(fields.get("call_type"), "call_type")),
        parse=parse,
    )


def model_group(fields: Object) -> Json:
    litellm_params = object_value(fields.get("litellm_params", {}), "litellm_params")
    return {"model_group": model_group_from(litellm_params)}


def strip_leading(fields: Object) -> Json:
    endpoint = string_value(fields.get("endpoint"), "endpoint")
    names = fields.get("model_names", [])
    if not isinstance(names, list):
        raise ValueError("model_names must be a list")
    model_names = tuple(string_value(name, "model_names[]") for name in names)
    return {"endpoint": strip_leading_model_segment(endpoint, model_names)}


def replace_segment(fields: Object) -> Json:
    return {
        "endpoint": replace_path_segment(
            string_value(fields.get("endpoint"), "endpoint"),
            string_value(fields.get("segment"), "segment"),
            string_value(fields.get("replacement"), "replacement"),
        )
    }


def relayed_object(fields: Object) -> Json:
    return {"relayed": cast(Json, relayed_json_object(relayed_response(fields)))}


def relayed_text(fields: Object) -> Json:
    return {"relayed": cast(Json, relayed_body(relayed_response(fields)))}


def relayed_shape(fields: Object) -> Json:
    shapes = fields.get("shapes", [])
    if not isinstance(shapes, list):
        raise ValueError("shapes must be a list")
    endpoint = string_value(fields.get("endpoint"), "endpoint")
    logging_obj = Logging()
    relayed = logged_relay_shape(
        [shape(value) for value in shapes],
        relayed_response(fields),
        cast(LiteLLMLoggingObj, logging_obj),
        endpoint,
    )
    return {"call_type": logging_obj.call_type, "relayed": cast(Json, relayed)}


OPERATIONS: Final[dict[str, Callable[[Object], Json]]] = {
    "model_group": model_group,
    "strip_leading_model_segment": strip_leading,
    "replace_path_segment": replace_segment,
    "relayed_json_object": relayed_object,
    "relayed_body": relayed_text,
    "logged_relay_shape": relayed_shape,
}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    operation = OPERATIONS.get(function)
    if operation is None:
        raise ValueError(f"unknown function {function}")
    return operation(input_value)
