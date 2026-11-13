from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.types.utils import ModelResponseStream

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Dumpable(Protocol):
    def model_dump(self) -> Json: ...


class Decoder(Protocol):
    current_content_block_type: str | None
    is_response_format_tool: bool
    converted_response_format_tool: bool
    tool_index: int
    served_model: str | None
    _current_server_tool_id: str | None
    _container_id: str | None
    _server_tool_inputs: dict[str, Json]

    def chunk_parser(self, *, chunk: dict[str, Json]) -> ModelResponseStream: ...

    def _handle_accumulated_json_chunk(
        self, *, data_str: str, is_final: bool
    ) -> ModelResponseStream | None: ...


class ChunkStep(TypedDict):
    function: Literal["chunk"]
    chunk: dict[str, Json]


class AccumulatedStep(TypedDict):
    function: Literal["accumulated"]
    fragment: str
    is_final: bool


class StateStep(TypedDict):
    function: Literal["state"]


Step: TypeAlias = ChunkStep | AccumulatedStep | StateStep


class Input(TypedDict):
    json_mode: bool | None
    tool_name_reverse_map: dict[str, str]
    ids: list[str]
    steps: list[Step]
