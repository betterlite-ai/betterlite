import json
from typing import Literal, Protocol, TypeAlias, cast

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.containers.transformation import BaseContainerConfig
from litellm.llms.custom_httpx import container_handler
from litellm.types.containers.main import (
    ContainerFileListResponse,
    ContainerFileObject,
    ContainerListResponse,
    ContainerObject,
    DeleteContainerFileResponse,
    DeleteContainerResult,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
RecordName = Literal[
    "container_object",
    "delete_result",
    "list_response",
    "file_object",
    "file_list_response",
    "delete_file_response",
]


class RecordModel(Protocol):
    def model_dump(self) -> object: ...

    def get(self, key: str, default: object = None) -> object: ...


class RecordBuilder(Protocol):
    def __call__(self, **fields: Json) -> RecordModel: ...


RECORD_TYPES = cast(
    "dict[RecordName, RecordBuilder]",
    {
        "container_object": ContainerObject,
        "delete_result": DeleteContainerResult,
        "list_response": ContainerListResponse,
        "file_object": ContainerFileObject,
        "file_list_response": ContainerFileListResponse,
        "delete_file_response": DeleteContainerFileResponse,
    },
)


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def as_object(value: Json) -> Object:
    return cast("Object", value)


def objects(value: Json) -> list[Object]:
    return cast("list[Object]", value)


def build(kind: str, value: Json) -> RecordModel:
    return RECORD_TYPES[cast("RecordName", kind)](**as_object(value))


def record_decode(records: Json) -> Json:
    return [
        to_json(build(cast(str, entry["kind"]), entry["value"]).model_dump())
        for entry in objects(records)
    ]


def record_field(record: Object, names: list[str]) -> Json:
    model = build(cast(str, record["kind"]), record["value"])
    return [to_json(model.get(name)) for name in names]


def endpoint_catalog() -> Json:
    document = cast("Object", container_handler._load_endpoints_config())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return [
        {
            "name": entry["name"],
            "async_name": entry["async_name"],
            "path": entry["path"],
            "method": entry["method"],
            "path_params": entry["path_params"],
            "query_params": entry["query_params"],
            "response_type": entry["response_type"],
            "is_multipart": entry.get("is_multipart", False),
            "returns_binary": entry.get("returns_binary", False),
        }
        for entry in objects(document["endpoints"])
    ]


def fixture_config() -> BaseContainerConfig:
    config_type = cast(
        "type[BaseContainerConfig]", type("Fixture", (BaseContainerConfig,), {})
    )
    setattr(config_type, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return config_type()


def api_base_outcomes(config: BaseContainerConfig, api_bases: list[str | None]) -> list[Json]:
    litellm_params: dict[str, object] = {}
    outcomes: list[Json] = []
    for api_base in api_bases:
        try:
            outcomes.append(
                BaseContainerConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                    config, api_base=api_base, litellm_params=litellm_params
                )
            )
        except ValueError:
            outcomes.append(None)
    return outcomes


def error_facts(config: BaseContainerConfig, error: Object) -> Json:
    headers = cast("dict[str, object]", error["headers"])
    try:
        BaseContainerConfig.get_error_class(  # pyright: ignore[reportUnknownMemberType]
            config,
            error_message=cast(str, error["message"]),
            status_code=cast(int, error["status_code"]),
            headers=headers,
        )
    except BaseLLMException as raised:
        return {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": to_json(cast("object", raised.headers)),
        }
    raise AssertionError("get_error_class returned without raising")


def config_defaults(fields: Object) -> Json:
    config = fixture_config()
    environment = cast(
        "Object",
        BaseContainerConfig.validate_environment(  # pyright: ignore[reportUnknownMemberType]
            config,
            headers=cast("dict[str, str]", fields["headers"]),
            api_key=cast("str | None", fields["api_key"]),
        ),
    )
    return {
        "environment": environment,
        "urls": api_base_outcomes(config, cast("list[str | None]", fields["api_bases"])),
        "error": error_facts(config, as_object(fields["error"])),
    }


def run(input_value: Object) -> Json:
    operation = input_value["operation"]
    if operation == "record_decode":
        return record_decode(input_value["records"])
    if operation == "record_field":
        return record_field(
            as_object(input_value["record"]), cast("list[str]", input_value["names"])
        )
    if operation == "endpoint_catalog":
        return endpoint_catalog()
    if operation == "config_defaults":
        return config_defaults(input_value)
    raise ValueError(f"unknown operation: {operation}")
