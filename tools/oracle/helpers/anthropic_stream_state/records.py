from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.types.responses.main import OutputCodeInterpreterCall
from litellm.types.utils import Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Dumpable(Protocol):
    def model_dump(self) -> Json: ...


class Decoder(Protocol):
    content_blocks: list[Json]
    tool_index: int
    json_mode: bool | None
    speed: str | None
    tool_name_reverse_map: dict[str, Json]
    response_id: str
    served_model: str | None
    is_response_format_tool: bool
    converted_response_format_tool: bool
    chunk_type: Literal["valid_json", "accumulated_json"]
    current_content_block_type: str | None
    web_search_results: list[Json]
    compaction_blocks: list[Json]
    reasoning_content_chunks: list[Json]
    tool_results: list[Json]
    accumulated_json: str
    _cumulative_usage: dict[str, Json]
    _web_search_calls: dict[str, Dumpable]
    _server_tool_inputs: dict[str, Json]
    _current_server_tool_id: str | None
    _container_id: str | None

    def get_content_block_start(self, chunk: dict[str, Json]) -> Json: ...

    def check_empty_tool_call_args(self) -> bool: ...

    def _content_block_delta_helper(
        self, chunk: dict[str, Json]
    ) -> tuple[str, Json, Json, Json, str | None]: ...

    def _handle_usage(self, anthropic_usage_chunk: dict[str, Json]) -> Usage: ...

    def _handle_message_delta(
        self, chunk: dict[str, Json]
    ) -> tuple[str, Usage | None, Json | None]: ...

    def _handle_redacted_thinking(
        self, content_block_start: Json, provider_specific_fields: dict[str, Json]
    ) -> tuple[Json, dict[str, Json]]: ...

    def _web_search_call_snapshot(self) -> dict[str, Dumpable]: ...

    def _complete_web_search_call(self, result: dict[str, Json]) -> None: ...

    def _build_interpreter_results(self) -> list[OutputCodeInterpreterCall]: ...


class StateFields(TypedDict, total=False):
    content_blocks: list[Json]
    web_search_calls: dict[str, Json]
    server_tool_inputs: dict[str, Json]
    tool_results: list[Json]
    container_id: str | None
    current_content_block_type: str | None
    tool_index: int
    converted_response_format_tool: bool
    accumulated_json: str


class SetupStep(TypedDict):
    function: Literal["setup"]
    fields: StateFields


class DeltaStep(TypedDict):
    function: Literal["delta"]
    chunk: dict[str, Json]


class BlockStartStep(TypedDict):
    function: Literal["block_start"]
    chunk: dict[str, Json]


class RedactedStep(TypedDict):
    function: Literal["redacted"]
    chunk: dict[str, Json]


class CheckEmptyStep(TypedDict):
    function: Literal["check_empty"]


class UsageStep(TypedDict):
    function: Literal["usage"]
    usage: dict[str, Json]


class MessageDeltaStep(TypedDict):
    function: Literal["message_delta"]
    chunk: dict[str, Json]


class WebSnapshotStep(TypedDict):
    function: Literal["web_snapshot"]


class CompleteWebSearchStep(TypedDict):
    function: Literal["complete_web_search"]
    result: dict[str, Json]


class CodeResultsStep(TypedDict):
    function: Literal["code_results"]


class StateStep(TypedDict):
    function: Literal["state"]


Step: TypeAlias = (
    SetupStep
    | DeltaStep
    | BlockStartStep
    | RedactedStep
    | CheckEmptyStep
    | UsageStep
    | MessageDeltaStep
    | WebSnapshotStep
    | CompleteWebSearchStep
    | CodeResultsStep
    | StateStep
)


class Input(TypedDict):
    json_mode: bool | None
    speed: str | None
    sync_stream: bool
    tool_name_reverse_map: dict[str, str]
    ids: list[str]
    steps: list[Step]
