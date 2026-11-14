from typing import Literal

from pydantic import JsonValue, PrivateAttr
from typing_extensions import TypedDict, override

from betterlite.types.utils.base import InitValue, ResponseObject

Phase = Literal["commentary", "final_answer"] | None


class GenericResponseOutputItemContentAnnotation(  # ast-grep-ignore: no-long-names
    ResponseObject,
):
    type: str | None
    start_index: int | None
    end_index: int | None
    url: str | None
    title: str | None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class OutputText(ResponseObject):
    type: str | None
    text: str | None
    # ast-grep-ignore: no-long-names
    annotations: list[GenericResponseOutputItemContentAnnotation] | None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class OutputFunctionToolCall(ResponseObject):
    arguments: str | None
    call_id: str | None
    name: str | None
    type: str | None
    id: str | None
    status: Literal["in_progress", "completed", "incomplete"]
    phase: Phase = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


def _json_field(value: JsonValue, name: str) -> JsonValue:
    match value:
        case dict():
            return value.get(name)
        case bool() | int() | float() | str() | list() | None:
            return None


def _text_field(value: JsonValue, name: str) -> str:
    match _json_field(value, name):
        case str() as text:
            return text
        case bool() | int() | float() | list() | dict() | None:
            return ""


def _web_search_source_url(item: JsonValue) -> str | None:
    if _text_field(item, "type") != "web_search_result":
        return None
    match _json_field(item, "url"):
        case str() as url:
            return url
        case bool() | int() | float() | list() | dict() | None:
            return None


def build_web_search_call(
    tool_id: str,
    tool_input: JsonValue,
    result: JsonValue,
    status: Literal["in_progress", "searching", "completed", "failed"] | None = None,
) -> ResponseObject:
    query = _text_field(tool_input, "query")
    content = _json_field(result, "content")
    sources: list[JsonValue] = []
    match content:
        case list():
            for item in content:
                url = _web_search_source_url(item)
                if url is not None:
                    sources.append({"type": "url", "url": url})
        case bool() | int() | float() | str() | dict() | None:
            pass
    failed = _text_field(content, "type") == "web_search_tool_result_error"
    resolved: Literal["in_progress", "searching", "completed", "failed"] = (
        "failed" if failed else "completed"
    )
    if status is not None:
        resolved = status
    return ResponseObject(
        id=f"ws_{tool_id}",
        type="web_search_call",
        status=resolved,
        action={
            "type": "search",
            "query": query,
            "queries": [query] if query != "" else [],
            "sources": sources,
        },
    )


class OutputImageGenerationCall(ResponseObject):
    type: Literal["image_generation_call"]
    id: str
    status: Literal["in_progress", "completed", "incomplete", "failed"]
    result: str | None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class OutputCodeInterpreterCallLog(ResponseObject):
    type: Literal["logs"]
    logs: str

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class OutputCodeInterpreterCall(ResponseObject):
    type: Literal["code_interpreter_call"]
    id: str
    code: str | None
    container_id: str | None
    status: Literal["in_progress", "completed", "incomplete", "failed"]
    outputs: list[OutputCodeInterpreterCallLog] | None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


def build_code_interpreter_log_outputs(  # ast-grep-ignore: no-long-names
    content: JsonValue,
) -> list[OutputCodeInterpreterCallLog] | None:
    match content:
        case dict():
            parts: list[str] = []
            stdout = _text_field(content, "stdout")
            if stdout != "":
                parts.append(stdout)
            stderr = _text_field(content, "stderr")
            if stderr != "":
                parts.append(f"STDERR: {stderr}")
            logs = "".join(parts)
            if logs == "":
                return None
            return [OutputCodeInterpreterCallLog(type="logs", logs=logs)]
        case bool() | int() | float() | str() | list() | None:
            return None


class CustomToolCallOutputItem(ResponseObject):
    type: Literal["custom_tool_call"]
    call_id: str
    id: str | None = None
    name: str
    input: str
    status: Literal["in_progress", "completed", "incomplete"] | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class GenericResponseOutputItem(ResponseObject):
    type: str
    id: str
    status: str
    role: str
    content: list[OutputText]
    phase: Phase = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class DeleteResponseResult(ResponseObject):
    id: str | None
    object: str | None
    deleted: bool | None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class DecodedResponseId(TypedDict, total=False):
    custom_llm_provider: str | None
    model_id: str | None
    response_id: str
