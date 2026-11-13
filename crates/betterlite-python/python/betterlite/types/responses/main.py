from typing import ClassVar, Literal

from typing_extensions import TypedDict, override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import JsonValue, StoredValue, allows_none

Phase = Literal["commentary", "final_answer"] | None


class RequiredFields(ResponseObject):
    # pydantic makes a nullable field without a default required; the base would default it to None
    @override
    def __init_subclass__(cls, **kwargs: StoredValue) -> None:
        super().__init_subclass__(**kwargs)
        nullable = {
            name: field for name, field in cls.__fields__.items() if allows_none(field)
        }
        cls.__defaults__ = {
            name: value
            for name, value in cls.__defaults__.items()
            if name not in nullable or name in cls.defaults
        }


class GenericResponseOutputItemContentAnnotation(  # ast-grep-ignore: no-long-names
    RequiredFields,
):
    type: str | None
    start_index: int | None
    end_index: int | None
    url: str | None
    title: str | None


class OutputText(RequiredFields):
    type: str | None
    text: str | None
    # ast-grep-ignore: no-long-names
    annotations: list[GenericResponseOutputItemContentAnnotation] | None


class OutputFunctionToolCall(RequiredFields):
    arguments: str | None
    call_id: str | None
    name: str | None
    type: str | None
    id: str | None
    status: Literal["in_progress", "completed", "incomplete"]
    phase: Phase
    defaults: ClassVar[dict[str, StoredValue]] = {"phase": None}


def _json_field(value: JsonValue, name: str) -> JsonValue:
    match value:
        case dict():
            return value.get(name)
        case _other:
            return None


def _text_field(value: JsonValue, name: str) -> str:
    match _json_field(value, name):
        case str() as text:
            return text
        case _other:
            return ""


def _web_search_source_url(item: JsonValue) -> str | None:
    if _text_field(item, "type") != "web_search_result":
        return None
    match _json_field(item, "url"):
        case str() as url:
            return url
        case _other:
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
        case _other:
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


class OutputImageGenerationCall(RequiredFields):
    type: Literal["image_generation_call"]
    id: str
    status: Literal["in_progress", "completed", "incomplete", "failed"]
    result: str | None


class OutputCodeInterpreterCallLog(RequiredFields):
    type: Literal["logs"]
    logs: str


class OutputCodeInterpreterCall(RequiredFields):
    type: Literal["code_interpreter_call"]
    id: str
    code: str | None
    container_id: str | None
    status: Literal["in_progress", "completed", "incomplete", "failed"]
    outputs: list[OutputCodeInterpreterCallLog] | None


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
        case _other:
            return None


class CustomToolCallOutputItem(RequiredFields):
    type: Literal["custom_tool_call"]
    call_id: str
    id: str | None
    name: str
    input: str
    status: Literal["in_progress", "completed", "incomplete"] | None
    defaults: ClassVar[dict[str, StoredValue]] = {"id": None, "status": None}


class GenericResponseOutputItem(RequiredFields):
    type: str
    id: str
    status: str
    role: str
    content: list[OutputText]
    phase: Phase
    defaults: ClassVar[dict[str, StoredValue]] = {"phase": None}


class DeleteResponseResult(RequiredFields):
    id: str | None
    object: str | None
    deleted: bool | None
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}


class DecodedResponseId(TypedDict, total=False):
    custom_llm_provider: str | None
    model_id: str | None
    response_id: str
