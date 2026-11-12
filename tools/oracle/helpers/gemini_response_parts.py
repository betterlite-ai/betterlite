import json
from typing import Final, Protocol, TypedDict, cast

from litellm._uuid import uuid as fastuuid
from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types import utils as litellm_utils
from litellm.types.llms.vertex_ai import LogprobsResult
from litellm.types.utils import ChatCompletionAudioResponse, ChoiceLogprobs
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON_VALUE: Final = TypeAdapter(JsonValue)


class Part(TypedDict, total=False):
    text: str
    inlineData: JsonValue
    functionCall: JsonValue
    toolCall: JsonValue
    toolResponse: JsonValue
    thought: bool
    thoughtSignature: str


class Function(TypedDict, total=False):
    arguments: str


class Tool(TypedDict):
    function: Function


class Input(TypedDict):
    parts: list[Part]
    legacy_function: bool
    cumulative_tool_call_idx: int
    now: int
    generated_id: str
    logprobs: LogprobsResult | None


class Config(Protocol):
    def get_assistant_content_message(self, parts: list[Part]) -> tuple[str | None, str | None]: ...

    # ast-grep-ignore: no-long-names
    def _extract_thinking_blocks_from_parts(self, parts: list[Part]) -> list[JsonValue]: ...

    # ast-grep-ignore: no-long-names
    def _extract_thought_signatures_from_parts(self, parts: list[Part]) -> list[str] | None: ...

    # ast-grep-ignore: no-long-names
    def _extract_server_side_tool_invocations(
        self, parts: list[Part]
    ) -> list[JsonValue] | None: ...

    # ast-grep-ignore: no-long-names
    def _extract_image_response_from_parts(self, parts: list[Part]) -> list[JsonValue] | None: ...

    # ast-grep-ignore: no-long-names
    def _extract_audio_response_from_parts(
        self, parts: list[Part]
    ) -> ChatCompletionAudioResponse | None: ...

    def _transform_parts(
        self, parts: list[Part], *, cumulative_tool_call_idx: int, is_function_call: bool
    ) -> tuple[Function | None, list[Tool] | None, int]: ...

    def _transform_logprobs(self, result: LogprobsResult | None) -> ChoiceLogprobs | None: ...


CONFIG: Final = cast(Config, VertexGeminiConfig())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1349


class _IdSource(Protocol):
    def uuid4(self) -> fastuuid.UUID: ...

    def time(self) -> float: ...


class _Fixed(_IdSource):
    def __init__(self, value: str, seconds: int) -> None:
        self._value = fastuuid.UUID(hex=value)
        self._seconds = seconds

    @override
    def uuid4(self) -> fastuuid.UUID:
        return self._value

    @override
    def time(self) -> float:
        return float(self._seconds)


def _inject(input_value: Input) -> None:
    fixed = _Fixed(input_value["generated_id"], input_value["now"])
    # LiteLLM 1.105.1 draws its synthetic tool ids and audio ids from these module attributes
    gemini.uuid = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1581
    litellm_utils.uuid = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1358
    gemini.time = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1524


def _audio(audio: ChatCompletionAudioResponse | None) -> JsonValue:
    if audio is None:
        return None
    return {
        "id": audio.id,
        "data": audio.data,
        "expires_at": audio.expires_at,
        "transcript": audio.transcript,
    }


def _compact(chunk: Function) -> None:
    arguments = chunk.get("arguments")
    if arguments is None:
        return
    parsed = JSON_VALUE.validate_json(arguments)
    chunk["arguments"] = json.dumps(parsed, ensure_ascii=False, separators=(",", ":"))


def _compact_tools(tools: list[Tool] | None) -> list[Tool] | None:
    if tools is not None:
        for tool in tools:
            _compact(tool["function"])
    return tools


def _logprobs(result: ChoiceLogprobs | None) -> JsonValue:
    if result is None or result.content is None:
        return None
    return {
        "content": [
            {
                "token": token.token,
                "logprob": token.logprob,
                "top_logprobs": [
                    {"token": top.token, "logprob": top.logprob} for top in token.top_logprobs
                ],
            }
            for token in result.content
        ]
    }


def run(input_value: Input) -> JsonValue:
    _inject(input_value)
    parts = input_value["parts"]
    content, reasoning_content = CONFIG.get_assistant_content_message(parts)
    function, tool_calls, next_tool_call_index = CONFIG._transform_parts(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1549
        parts,
        cumulative_tool_call_idx=input_value["cumulative_tool_call_idx"],
        is_function_call=input_value["legacy_function"],
    )
    if function is not None:
        _compact(function)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1349
        JsonValue,
        {
            "content": content,
            "reasoning_content": reasoning_content,
            "thinking_blocks": CONFIG._extract_thinking_blocks_from_parts(parts),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1390
            "thought_signatures": CONFIG._extract_thought_signatures_from_parts(parts),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1413
            "server_side_tool_invocations": CONFIG._extract_server_side_tool_invocations(parts),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1431
            "images": CONFIG._extract_image_response_from_parts(parts),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1492
            "audio": _audio(CONFIG._extract_audio_response_from_parts(parts)),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1512
            "function": function,
            "tool_calls": _compact_tools(tool_calls),
            "next_tool_call_index": next_tool_call_index,
            "logprobs": _logprobs(CONFIG._transform_logprobs(input_value["logprobs"])),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1606
        },
    )
