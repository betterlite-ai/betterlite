from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.openai.containers.utils import join_container_api_base_path
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes
from pydantic import JsonValue, ValidationError

from .cases import (
    ContentCall,
    CreateCall,
    ErrorCall,
    FileListCall,
    HeadersCall,
    JoinCall,
    ListCall,
    MapCall,
    MetadataCall,
    MetadataKind,
    ReplyCall,
    ResourceCall,
    UrlCall,
)
from .contracts import (
    BODY,
    CREATE_PARAMS,
    HIDDEN,
    JSON,
    ContainerConfig,
    environment,
    globals_applied,
)

STARTED: Final = datetime(2020, 1, 1, tzinfo=UTC)


def recorder() -> Logging:
    return Logging(
        model="openai/container",
        messages=None,
        stream=False,
        call_type=CallTypes.create_container,
        start_time=STARTED,
        litellm_call_id="b522",
        function_id="b522",
    )


def map_params(config: ContainerConfig, call: MapCall) -> JsonValue:
    params = CREATE_PARAMS.validate_python(call["params"])
    mapped = config.map_openai_params(params, drop_params=call["drop_params"])
    answer: dict[str, JsonValue] = {"mapped": JSON.validate_python(mapped)}
    return JSON.validate_python(answer)


def join_path(call: JoinCall) -> JsonValue:
    joined = join_container_api_base_path(call["api_base"], call["path_suffix"])
    answer: dict[str, JsonValue] = {"url": joined}
    return JSON.validate_python(answer)


def url(config: ContainerConfig, call: UrlCall) -> JsonValue:
    with environment(call["environment"]), globals_applied(call["globals"]):
        resolved = config.get_complete_url(call["api_base"], {})
    answer: dict[str, JsonValue] = {"url": resolved}
    return JSON.validate_python(answer)


def create(config: ContainerConfig, call: CreateCall) -> JsonValue:
    params = BODY.validate_python(call["params"])
    request = config.transform_container_create_request(
        call["name"], params, GenericLiteLLMParams(), {}
    )
    answer: dict[str, JsonValue] = {"request": JSON.validate_python(request)}
    return JSON.validate_python(answer)


def list_request(config: ContainerConfig, call: ListCall) -> JsonValue:
    endpoint, query = config.transform_container_list_request(
        call["api_base"],
        GenericLiteLLMParams(),
        {},
        after=call["after"],
        limit=call["limit"],
        order=call["order"],
        extra_query=call["extra_query"],
    )
    answer: dict[str, JsonValue] = {"url": endpoint, "params": JSON.validate_python(query)}
    return JSON.validate_python(answer)


def file_list_request(config: ContainerConfig, call: FileListCall) -> JsonValue:
    endpoint, query = config.transform_container_file_list_request(
        call["container_id"],
        call["api_base"],
        GenericLiteLLMParams(),
        {},
        after=call["after"],
        limit=call["limit"],
        order=call["order"],
        extra_query=call["extra_query"],
    )
    answer: dict[str, JsonValue] = {"url": endpoint, "params": JSON.validate_python(query)}
    return JSON.validate_python(answer)


def resource(config: ContainerConfig, call: ResourceCall) -> JsonValue:
    endpoint, data = (
        config.transform_container_retrieve_request(
            call["container_id"], call["api_base"], GenericLiteLLMParams(), {}
        )
        if call["kind"] == "retrieve"
        else config.transform_container_delete_request(
            call["container_id"], call["api_base"], GenericLiteLLMParams(), {}
        )
    )
    answer: dict[str, JsonValue] = {"url": endpoint, "data": JSON.validate_python(data)}
    return JSON.validate_python(answer)


def content(config: ContainerConfig, call: ContentCall) -> JsonValue:
    endpoint, query = config.transform_container_file_content_request(
        call["container_id"], call["file_id"], call["api_base"], GenericLiteLLMParams(), {}
    )
    answer: dict[str, JsonValue] = {"url": endpoint, "params": JSON.validate_python(query)}
    return JSON.validate_python(answer)


def headers(config: ContainerConfig, call: HeadersCall) -> JsonValue:
    with environment(call["environment"]), globals_applied(call["globals"]):
        resolved = config.validate_environment(dict(call["headers"]), call["api_key"])
    answer: dict[str, JsonValue] = {"headers": JSON.validate_python(resolved)}
    return JSON.validate_python(answer)


def create_reply(config: ContainerConfig, call: ReplyCall) -> JsonValue:
    raw = httpx.Response(200, content=call["body"].encode())
    try:
        record = config.transform_container_create_response(raw, recorder())
    except ValidationError:
        answer: dict[str, JsonValue] = {"ok": False, "record": None, "hidden": None}
        return JSON.validate_python(answer)
    projected: dict[str, JsonValue] = {
        "ok": True,
        "record": JSON.validate_python(record.model_dump()),
        "hidden": HIDDEN.validate_python(record.get("_hidden_params")),
    }
    return JSON.validate_python(projected)


def decode_metadata(config: ContainerConfig, kind: MetadataKind, raw: httpx.Response) -> JsonValue:
    if kind == "retrieve":
        retrieved = config.transform_container_retrieve_response(raw, recorder())
        return JSON.validate_python(retrieved.model_dump())
    if kind == "list":
        listed = config.transform_container_list_response(raw, recorder())
        return JSON.validate_python(listed.model_dump())
    if kind == "delete":
        deleted = config.transform_container_delete_response(raw, recorder())
        return JSON.validate_python(deleted.model_dump())
    files = config.transform_container_file_list_response(raw, recorder())
    return JSON.validate_python(files.model_dump())


def metadata_reply(config: ContainerConfig, call: MetadataCall) -> JsonValue:
    raw = httpx.Response(200, content=call["body"].encode())
    try:
        record = decode_metadata(config, call["kind"], raw)
    except ValidationError:
        answer: dict[str, JsonValue] = {"ok": False, "record": None}
        return JSON.validate_python(answer)
    projected: dict[str, JsonValue] = {"ok": True, "record": record}
    return JSON.validate_python(projected)


def content_reply(config: ContainerConfig, call: ReplyCall) -> JsonValue:
    raw = httpx.Response(200, content=call["body"].encode())
    payload = config.transform_container_file_content_response(raw, recorder())
    answer: dict[str, JsonValue] = {"content": payload.decode()}
    return JSON.validate_python(answer)


def error(config: ContainerConfig, call: ErrorCall) -> JsonValue:
    try:
        config.get_error_class(call["message"], call["status"], dict(call["headers"]))
    except BaseLLMException as raised:
        answer: dict[str, JsonValue] = {
            "status_code": raised.status_code,
            "message": raised.message,
        }
        return JSON.validate_python(answer)
    raise RuntimeError("get_error_class returned without raising")
