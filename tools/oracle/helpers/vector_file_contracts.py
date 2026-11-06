import json
from typing import Literal, Protocol, TypeAlias, cast

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.vector_store_files.transformation import BaseVectorStoreFilesConfig
from litellm.types.vector_store_files import (
    VectorStoreFileAuthCredentials,
    VectorStoreFileChunkingStrategy,
    VectorStoreFileContentResponse,
    VectorStoreFileContentTextPart,
    VectorStoreFileCreateRequest,
    VectorStoreFileDeleteResponse,
    VectorStoreFileListQueryParams,
    VectorStoreFileListResponse,
    VectorStoreFileObject,
    VectorStoreFileStaticChunkingConfig,
    VectorStoreFileUpdateRequest,
)
from litellm.vector_store_files.utils import VectorStoreFileRequestUtils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Operation = Literal["create", "list", "read", "update", "delete"]
OPERATIONS: tuple[Operation, ...] = ("create", "list", "read", "update", "delete")


class RecordBuilder(Protocol):
    def __call__(self, **fields: Json) -> Json: ...


RECORD_TYPES = cast(
    "dict[str, RecordBuilder]",
    {
        "file_object": VectorStoreFileObject,
        "create_request": VectorStoreFileCreateRequest,
        "update_request": VectorStoreFileUpdateRequest,
        "list_query": VectorStoreFileListQueryParams,
        "list_response": VectorStoreFileListResponse,
        "delete_response": VectorStoreFileDeleteResponse,
        "content_response": VectorStoreFileContentResponse,
        "content_text_part": VectorStoreFileContentTextPart,
        "auth_credentials": VectorStoreFileAuthCredentials,
        "chunking_strategy": VectorStoreFileChunkingStrategy,
        "static_chunking": VectorStoreFileStaticChunkingConfig,
    },
)


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def objects(value: Json) -> list[Object]:
    return cast("list[Object]", value)


def as_object(value: Json) -> Object:
    return cast("Object", value)


def fixture() -> BaseVectorStoreFilesConfig:
    config_type = cast(
        "type[BaseVectorStoreFilesConfig]", type("Fixture", (BaseVectorStoreFilesConfig,), {})
    )
    setattr(config_type, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return config_type()


def api_base_outcome(config: BaseVectorStoreFilesConfig, api_base: str | None) -> Json:
    try:
        return config.get_complete_url(api_base=api_base, vector_store_id="vs_a", litellm_params={})
    except ValueError as error:
        return {"error": {"class": type(error).__name__, "message": str(error)}}


def error_facts(config: BaseVectorStoreFilesConfig, error: Object) -> Json:
    headers = cast("dict[str, object]", error["headers"])
    try:
        config.get_error_class(
            error_message=cast("str", error["message"]),
            status_code=cast("int", error["status_code"]),
            headers=headers,
        )
    except BaseLLMException as raised:
        raised_headers = cast("object", raised.headers)  # pyright: ignore[reportUnknownMemberType]
        return {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": to_json(raised_headers),
        }
    raise AssertionError("get_error_class returned without raising")


def provider_defaults(fields: Object) -> Json:
    config = fixture()
    optional_params = cast("dict[str, object]", fields["optional_params"])
    non_default_params = cast("dict[str, object]", fields["non_default_params"])
    request_headers = cast("dict[str, str]", fields["headers"])
    signed_headers, body = config.sign_request(
        headers=request_headers,
        optional_params=optional_params,
        request_data=cast("dict[str, object]", fields["request_data"]),
        api_base=cast("str", fields["api_base"]),
        api_key=cast("str | None", fields["api_key"]),
    )
    return to_json(
        {
            "supported": {
                name: list(config.get_supported_openai_params(name)) for name in OPERATIONS
            },
            "mapped": {
                str(drop_params): dict(
                    config.map_openai_params(
                        operation="list",
                        non_default_params=non_default_params,
                        optional_params=optional_params,
                        drop_params=drop_params,
                    )
                )
                for drop_params in (True, False)
            },
            "environment": config.validate_environment(
                headers=request_headers, litellm_params=None
            ),
            "urls": [
                api_base_outcome(config, api_base)
                for api_base in cast("list[str | None]", fields["api_bases"])
            ],
            "signed_headers": signed_headers,
            "signed_body": None if body is None else list(body),
            "error": error_facts(config, as_object(fields["error"])),
        }
    )


def run(fields: Object) -> Json:
    operation = fields["operation"]
    if operation == "attachment_parameter_filters":
        return {
            "create": [
                to_json(VectorStoreFileRequestUtils.get_create_request_params(params))
                for params in objects(fields["create"])
            ],
            "update": [
                to_json(VectorStoreFileRequestUtils.get_update_request_params(params))
                for params in objects(fields["update"])
            ],
        }
    if operation == "list_query_filter":
        return [
            to_json(VectorStoreFileRequestUtils.get_list_query_params(params))
            for params in objects(fields["queries"])
        ]
    if operation == "attachment_record_fields":
        return [
            RECORD_TYPES[cast("str", entry["kind"])](**as_object(entry["value"]))
            for entry in objects(fields["records"])
        ]
    if operation == "file_provider_defaults":
        return provider_defaults(fields)
    raise ValueError(f"unknown operation: {operation}")
