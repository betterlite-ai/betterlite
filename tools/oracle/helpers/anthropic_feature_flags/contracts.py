from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.llms.anthropic import common_utils
from litellm.llms.anthropic.common_utils import AnthropicModelInfo
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]


class FeatureModelInfo(Protocol):
    def is_file_id_used(self, messages: list[JsonObject]) -> bool: ...

    def is_pdf_used(self, messages: list[JsonObject]) -> bool: ...

    def is_tool_search_used(self, tools: list[JsonObject] | None) -> bool: ...

    def is_input_examples_used(self, tools: list[JsonObject] | None) -> bool: ...

    def is_eager_input_streaming_used(self, tools: list[JsonObject] | None) -> bool: ...

    def is_computer_tool_used(self, tools: list[JsonObject] | None) -> str | None: ...

    def get_computer_tool_beta_header(self, computer_tool_version: str) -> str: ...

    def is_web_search_tool_used(self, tools: list[JsonObject] | None) -> bool: ...

    def is_code_execution_tool_used(self, tools: list[JsonObject] | None) -> bool: ...

    def is_container_with_skills_used(
        self, optional_params: JsonObject | None
    ) -> bool: ...

    def is_mcp_server_used(self, mcp_servers: list[JsonObject] | None) -> bool: ...



class OneShotPredicate(Protocol):
    def __call__(
        self,
        messages: list[JsonObject],
        system: Json,
        tools: Json,
        user_agent: str | None,
    ) -> bool: ...


MODEL_INFO: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L319
    FeatureModelInfo, AnthropicModelInfo()
)
PROGRAMMATIC_PREDICATE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L420
    "Callable[[list[JsonObject] | None], bool]",
    AnthropicModelInfo().is_programmatic_tool_calling_used,
)
USER_HEADERS: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L862
    "Callable[[str | None], list[str] | None]",
    AnthropicModelInfo()._get_user_anthropic_beta_headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L862
)
THINKING_DISPLAY: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L344
    "Callable[[Json | None], bool]",
    AnthropicModelInfo().is_thinking_display_updates_used,
)
TOOL_CHANGE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L349
    "Callable[[list[JsonObject]], bool]",
    AnthropicModelInfo().is_mid_conversation_tool_change_used,
)
OUTPUT_CONFIG: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L361
    "Callable[[list[JsonObject]], bool]",
    AnthropicModelInfo().is_mid_conversation_output_config_used,
)
class BetaList(Protocol):
    def __call__(
        self,
        model: str,
        optional_params: JsonObject | None,
        computer_tool_used: str | None,
        **flags: Json,
    ) -> list[str]: ...


BETA_LIST: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L886
    BetaList, AnthropicModelInfo().get_anthropic_beta_list
)
ONE_SHOT: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L196
    OneShotPredicate, common_utils.is_claude_code_one_shot_subagent_request
)


class Fixture(TypedDict):
    model_cost: JsonObject


class OneShotCall(TypedDict):
    function: Literal["one_shot"]
    user_agent: str | None
    messages: list[JsonObject]
    system: Json
    tools: Json


class BillingFieldsCall(TypedDict):
    function: Literal["billing_fields"]
    texts: list[str]


class CompactionCall(TypedDict):
    function: Literal["compaction"]
    provider: str
    params: JsonObject
    messages: list[JsonObject]


class CacheCall(TypedDict):
    function: Literal["cache_control"]
    model: str
    custom_llm_provider: str | None


class FileIdsCall(TypedDict):
    function: Literal["file_ids"]
    messages: list[JsonObject]


class PdfCall(TypedDict):
    function: Literal["pdf"]
    messages: list[JsonObject]


class ToolsCall(TypedDict):
    function: Literal["tools"]
    tools: list[JsonObject] | None


class EagerFlagCall(TypedDict):
    function: Literal["eager_flag"]
    tool: Json


class EagerUsedCall(TypedDict):
    function: Literal["eager_used"]
    tools: list[JsonObject] | None


class ComputerCall(TypedDict):
    function: Literal["computer"]
    tools: list[JsonObject] | None
    versions: list[str]


class SkillsCall(TypedDict):
    function: Literal["skills"]
    tools: list[JsonObject] | None
    params: JsonObject
    headers: list[str | None]
    providers: list[str]


class ConversationCall(TypedDict):
    function: Literal["conversation"]
    messages: list[JsonObject]
    thinking: Json
    mcp_servers: list[JsonObject] | None


FeatureName: TypeAlias = Literal[
    "file_id",
    "mcp_server",
    "output_config",
    "thinking_display",
    "tool_change",
]


class BetaCall(TypedDict):
    function: Literal["beta"]
    model: str
    custom_llm_provider: str
    params: JsonObject
    computer_tool_used: str | None
    prompt_caching_set: bool
    features: list[FeatureName]


RequestCall: TypeAlias = (
    OneShotCall
    | BillingFieldsCall
    | CompactionCall
    | CacheCall
    | FileIdsCall
    | PdfCall
)

ToolCall: TypeAlias = (
    ToolsCall
    | EagerFlagCall
    | EagerUsedCall
    | ComputerCall
    | SkillsCall
    | ConversationCall
)

BetaGroupCall: TypeAlias = BetaCall

Call: TypeAlias = RequestCall | ToolCall | BetaGroupCall


class Input(TypedDict):
    fixture: Fixture
    calls: list[Call]
