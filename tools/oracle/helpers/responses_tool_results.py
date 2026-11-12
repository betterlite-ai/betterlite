import json
from collections.abc import Iterator
from typing import Literal, TypeAlias, TypedDict

from litellm.responses.litellm_completion_transformation.transformation import (
    TOOL_CALLS_CACHE,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from openai.types.responses.response_apply_patch_tool_call import (
    OperationCreateFile,
    OperationDeleteFile,
    OperationUpdateFile,
    ResponseApplyPatchToolCall,
)
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue


class CacheEntry(TypedDict):
    key: str
    value: Json


class CachedCall(TypedDict):
    item: dict[str, Json]
    cache: list[CacheEntry]


class Identity(TypedDict):
    id: str | None
    call_id: str | None


class PlainProvider(TypedDict):
    kind: Literal["plain"]
    fields: dict[str, Json]


class RecordProvider(TypedDict):
    kind: Literal["record"]
    fields: dict[str, Json]


Provider: TypeAlias = PlainProvider | RecordProvider


class Function(TypedDict):
    name: str
    arguments: str
    id: str | None
    call_id: str | None
    provider: Provider | None


class FileOperation(TypedDict):
    type: Literal["create_file", "update_file"]
    path: str
    diff: str


class DeleteOperation(TypedDict):
    type: Literal["delete_file"]
    path: str


Operation: TypeAlias = FileOperation | DeleteOperation


class Patch(TypedDict):
    call_id: str
    operation: Operation
    index: int


class OutputInput(TypedDict):
    operation: Literal["tool_output_content"]
    items: list[dict[str, Json]]


class FunctionCallInput(TypedDict):
    operation: Literal["input_function_call"]
    items: list[dict[str, Json]]


class CachedInput(TypedDict):
    operation: Literal["cached_call_output"]
    calls: list[CachedCall]


class IdentityInput(TypedDict):
    operation: Literal["output_call_identity"]
    identities: list[Identity]
    functions: list[Function]
    patches: list[Patch]


Input: TypeAlias = OutputInput | FunctionCallInput | CachedInput | IdentityInput


class Record:
    def __init__(self, fields: dict[str, Json]) -> None:
        self.fields = fields

    def __iter__(self) -> Iterator[tuple[str, Json]]:
        return iter(self.fields.items())


class FunctionItem:
    def __init__(
        self, function: Function, provider_fields: dict[str, Json] | Record | None
    ) -> None:
        self.id = function["id"]
        self.call_id = function["call_id"]
        self.name = function["name"]
        self.arguments = function["arguments"]
        self.provider_specific_fields = provider_fields


def provider_fields(provider: Provider | None) -> dict[str, Json] | Record | None:
    if provider is None:
        return None
    if provider["kind"] == "plain":
        return provider["fields"]
    return Record(provider["fields"])


def apply_patch_item(call_id: str, operation: Operation) -> ResponseApplyPatchToolCall:
    if operation["type"] == "create_file":
        created = OperationCreateFile(
            type="create_file", path=operation["path"], diff=operation["diff"]
        )
        return ResponseApplyPatchToolCall(
            id="ap",
            call_id=call_id,
            operation=created,
            status="completed",
            type="apply_patch_call",
        )
    if operation["type"] == "update_file":
        updated = OperationUpdateFile(
            type="update_file", path=operation["path"], diff=operation["diff"]
        )
        return ResponseApplyPatchToolCall(
            id="ap",
            call_id=call_id,
            operation=updated,
            status="completed",
            type="apply_patch_call",
        )
    deleted = OperationDeleteFile(type="delete_file", path=operation["path"])
    return ResponseApplyPatchToolCall(
        id="ap",
        call_id=call_id,
        operation=deleted,
        status="completed",
        type="apply_patch_call",
    )


def run(input_value: Input) -> Json:
    wire = TypeAdapter(JsonValue)
    tool_output = ResponsesConfig._transform_responses_api_tool_call_output_to_chat_completion_message  # type: ignore[misc]  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1553
    function_call = ResponsesConfig._transform_responses_api_function_call_to_chat_completion_message  # type: ignore[misc]  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1689
    call_identity = ResponsesConfig._tool_call_id_from_responses_item  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2307
    if input_value["operation"] == "tool_output_content":
        converted = [tool_output(item) for item in input_value["items"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1553
        return wire.validate_json(json.dumps(converted))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1553
    if input_value["operation"] == "input_function_call":
        calls = [function_call(item) for item in input_value["items"]]  # type: ignore[misc, arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1689
        return wire.validate_json(json.dumps(calls))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1689
    if input_value["operation"] == "cached_call_output":
        outputs: list[Json] = []
        for call in input_value["calls"]:
            for entry in call["cache"]:
                TOOL_CALLS_CACHE.set_cache(entry["key"], entry["value"])  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L158
            outputs.append(wire.validate_json(json.dumps(tool_output(call["item"]))))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1553
            for entry in call["cache"]:
                TOOL_CALLS_CACHE.delete_cache(entry["key"])  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L268
        return outputs
    identity = {
        "ids": [
            call_identity(entry["id"], entry["call_id"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2307
            for entry in input_value["identities"]
        ],
        "functions": [
            ResponsesConfig.convert_response_function_tool_call_to_chat_completion_tool_call(
                FunctionItem(function, provider_fields(function["provider"])), 4
            )
            for function in input_value["functions"]
        ],
        "patches": [
            ResponsesConfig.convert_apply_patch_tool_call_to_chat_completion_tool_call(
                apply_patch_item(patch["call_id"], patch["operation"]), patch["index"]
            )
            for patch in input_value["patches"]
        ],
    }
    return wire.validate_json(json.dumps(identity))
