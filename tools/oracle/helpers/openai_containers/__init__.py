import os
from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Final, Protocol, TypeAlias

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.openai.containers.utils import join_container_api_base_path
from litellm.types.containers.main import ContainerCreateOptionalRequestParams as CreateParams
from litellm.types.containers.main import (
    ContainerFileListResponse,
    ContainerListResponse,
    ContainerObject,
    DeleteContainerResult,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

from .cases import (
    BodyCall,
    ErrorCall,
    Globals,
    HeadersCall,
    Input,
    ReplyCall,
    ReplyKind,
    RequestCall,
    UrlCall,
)

Params: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Prepared: TypeAlias = tuple[str, Params]
ListRequest: TypeAlias = Callable[
    [str, GenericLiteLLMParams, Headers, str | None, int | None, str | None, Params | None],
    Prepared,
]
FileListRequest: TypeAlias = Callable[
    [str, str, GenericLiteLLMParams, Headers, str | None, int | None, str | None, Params | None],
    Prepared,
]
ResourceRequest: TypeAlias = Callable[[str, str, GenericLiteLLMParams, Headers], Prepared]
ContentRequest: TypeAlias = Callable[[str, str, str, GenericLiteLLMParams, Headers], Prepared]

JSON: Final = TypeAdapter(JsonValue)
HIDDEN: Final = TypeAdapter(dict[str, JsonValue])
STARTED: Final = datetime(2020, 1, 1, tzinfo=UTC)
ENVIRONMENT: Final = ("OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_API_BASE", "OPENAI_ORGANIZATION")
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


Record: TypeAlias = (
    ContainerObject | ContainerListResponse | DeleteContainerResult | ContainerFileListResponse
)
Reply: TypeAlias = Callable[[httpx.Response, Logging], Record]


class ContainerConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Callable[[], list[str]]: ...

    @property
    def map_openai_params(self) -> Callable[[CreateParams, bool], Params]: ...

    @property
    def validate_environment(self) -> Callable[[Headers, str | None], Headers]: ...

    @property
    def get_complete_url(self) -> Callable[[str | None, Params], str]: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_create_request(
        self,
    ) -> Callable[[str, Params, GenericLiteLLMParams, Headers], Params]: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_create_response(self) -> Reply: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_list_request(self) -> ListRequest: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_list_response(self) -> Reply: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_retrieve_request(self) -> ResourceRequest: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_retrieve_response(self) -> Reply: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_delete_request(self) -> ResourceRequest: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_delete_response(self) -> Reply: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_file_list_request(self) -> FileListRequest: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_file_list_response(self) -> Reply: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_file_content_request(self) -> ContentRequest: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_container_file_content_response(
        self,
    ) -> Callable[[httpx.Response, Logging], bytes]: ...

    @property
    def get_error_class(self) -> Callable[[str, int, Headers], Exception]: ...


class ErrorRecord(BaseModel):
    status_code: int
    message: str
    headers: dict[str, str]


def container_config() -> ContainerConfig:
    found = ProviderConfigManager.get_provider_container_config(LlmProviders.OPENAI)
    if found is None:
        raise ValueError("LiteLLM registers no OpenAI container config")
    return found


def apply(environment: Mapping[str, str], values: Globals) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    os.environ.update(environment)
    litellm.api_base = values["api_base"]
    litellm.api_key = values["api_key"]
    litellm.openai_key = values["openai_key"]


def url(config: ContainerConfig, call: UrlCall) -> JsonValue:
    apply(call["environment"], call["globals"])
    answer: dict[str, JsonValue] = {"url": config.get_complete_url(call["api_base"], {})}
    return answer


def request(config: ContainerConfig, call: RequestCall) -> JsonValue:
    params = GenericLiteLLMParams()
    if call["kind"] == "list":
        query = call["query"]
        endpoint, sent = config.transform_container_list_request(
            call["api_base"],
            params,
            {},
            query["after"],
            query["limit"],
            query["order"],
            query["extra_query"],
        )
    elif call["kind"] == "file_list":
        query = call["query"]
        endpoint, sent = config.transform_container_file_list_request(
            call["container_id"],
            call["api_base"],
            params,
            {},
            query["after"],
            query["limit"],
            query["order"],
            query["extra_query"],
        )
    elif call["kind"] == "content":
        endpoint, sent = config.transform_container_file_content_request(
            call["container_id"], call["file_id"], call["api_base"], params, {}
        )
    elif call["kind"] == "retrieve":
        endpoint, sent = config.transform_container_retrieve_request(
            call["container_id"], call["api_base"], params, {}
        )
    else:
        endpoint, sent = config.transform_container_delete_request(
            call["container_id"], call["api_base"], params, {}
        )
    answer: dict[str, JsonValue] = {"url": endpoint, "params": sent}
    return answer


def headers(config: ContainerConfig, call: HeadersCall) -> JsonValue:
    apply(call["environment"], call["globals"])
    resolved = config.validate_environment(dict(call["headers"]), call["api_key"])
    answer: dict[str, JsonValue] = {"headers": JSON.validate_python(resolved)}
    return answer


def reply(config: ContainerConfig, call: ReplyCall, logging: Logging) -> JsonValue:
    decoders: dict[ReplyKind, Reply] = {
        "create": config.transform_container_create_response,
        "retrieve": config.transform_container_retrieve_response,
        "list": config.transform_container_list_response,
        "delete": config.transform_container_delete_response,
        "file_list": config.transform_container_file_list_response,
    }
    raw = httpx.Response(200, content=call["body"].encode())
    try:
        record = decoders[call["kind"]](raw, logging)
    except ValidationError:
        failed: dict[str, JsonValue] = {"ok": False}
        return failed
    dumped = JSON.validate_json(record.model_dump_json())
    answer: dict[str, JsonValue] = {"ok": True, "record": dumped}
    if call["kind"] == "create":
        answer["hidden"] = HIDDEN.validate_python(record.get("_hidden_params"))
    return answer


def content_reply(config: ContainerConfig, call: BodyCall, logging: Logging) -> JsonValue:
    raw = httpx.Response(200, content=call["body"].encode())
    content = config.transform_container_file_content_response(raw, logging)
    answer: dict[str, JsonValue] = {"content": content.decode()}
    return answer


def error(config: ContainerConfig, call: ErrorCall) -> JsonValue:
    try:
        config.get_error_class(call["message"], call["status"], dict(call["headers"]))
    except PROVIDER_ERROR as raised:
        record = ErrorRecord.model_validate(raised, from_attributes=True)
        return JSON.validate_json(record.model_dump_json())
    raise RuntimeError("get_error_class returned without raising")


def run(input_value: Input) -> JsonValue:
    config = container_config()
    logging = Logging(
        model="openai/container",
        messages=None,
        stream=False,
        call_type=CallTypes.create_container,
        start_time=STARTED,
        litellm_call_id="b522",
        function_id="b522",
    )
    supported: list[str] = []
    if input_value["supported"]:
        supported = config.get_supported_openai_params()
    answer: dict[str, JsonValue] = {"supported": JSON.validate_python(supported)}
    answer["maps"] = JSON.validate_python(
        [
            config.map_openai_params(call["params"], call["drop_params"])
            for call in input_value["maps"]
        ]
    )
    answer["joins"] = JSON.validate_python(
        [
            join_container_api_base_path(call["api_base"], call["path_suffix"])
            for call in input_value["joins"]
        ]
    )
    answer["urls"] = [url(config, call) for call in input_value["urls"]]
    answer["creates"] = JSON.validate_python(
        [
            config.transform_container_create_request(
                call["name"], call["params"], GenericLiteLLMParams(), {}
            )
            for call in input_value["creates"]
        ]
    )
    answer["requests"] = [request(config, call) for call in input_value["requests"]]
    answer["headers"] = [headers(config, call) for call in input_value["headers"]]
    answer["replies"] = [reply(config, call, logging) for call in input_value["replies"]]
    answer["content_replies"] = [
        content_reply(config, call, logging) for call in input_value["content_replies"]
    ]
    answer["errors"] = [error(config, call) for call in input_value["errors"]]
    return JSON.validate_python(answer)
