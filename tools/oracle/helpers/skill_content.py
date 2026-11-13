import base64
import logging
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

from litellm.llms.litellm_proxy.skills.code_execution import (
    get_litellm_code_execution_tool,
    get_litellm_code_execution_tool_anthropic,
)
from litellm.llms.litellm_proxy.skills.prompt_injection import SkillPromptInjectionHandler
from litellm.models.skills import LiteLLM_SkillsTable
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]


class SkillRow(TypedDict):
    skill_id: str
    display_title: str | None
    description: str | None
    instructions: str | None
    file_content: str | None
    metadata: JsonObject | None


class ExtractContentCall(TypedDict):
    function: Literal["extract_content"]
    skill: SkillRow


class ExtractFilesCall(TypedDict):
    function: Literal["extract_files"]
    skill: SkillRow


class ConvertToolCall(TypedDict):
    function: Literal["convert_tool"]
    skill: SkillRow


class ConvertAnthropicCall(TypedDict):
    function: Literal["convert_anthropic_tool"]
    skill: SkillRow


class InjectCall(TypedDict):
    function: Literal["inject"]
    data: JsonObject
    contents: list[str]
    use_anthropic_format: bool


class ExecutionToolCall(TypedDict):
    function: Literal["execution_tool"]


class ExecutionToolAnthropicCall(TypedDict):
    function: Literal["execution_tool_anthropic"]


Input: TypeAlias = (
    ExtractContentCall
    | ExtractFilesCall
    | ConvertToolCall
    | ConvertAnthropicCall
    | InjectCall
    | ExecutionToolCall
    | ExecutionToolAnthropicCall
)

class Injector(Protocol):
    # LiteLLM declares bare dict for this method, so the protocol names the records the case reads
    def inject_skill_content_to_messages(  # ast-grep-ignore: no-long-names
        self,
        data: dict[str, Json],
        skill_contents: list[str],
        *,
        use_anthropic_format: bool = False,
    ) -> dict[str, Json]: ...


HANDLER: Final = SkillPromptInjectionHandler()
INJECTOR: Injector = HANDLER
ANSWER: Final = TypeAdapter(Json)
# LiteLLM declares dict[str, Any] for both tool builders, so this binds them to a JSON object
CONVERT: Final[Callable[[LiteLLM_SkillsTable], JsonObject]] = HANDLER.convert_skill_to_tool
CONVERT_ANTHROPIC: Final[Callable[[LiteLLM_SkillsTable], JsonObject]] = (
    HANDLER.convert_skill_to_anthropic_tool
)


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def build_skill(row: SkillRow) -> LiteLLM_SkillsTable:
    content = row["file_content"]
    return LiteLLM_SkillsTable(
        skill_id=row["skill_id"],
        display_title=row["display_title"],
        description=row["description"],
        instructions=row["instructions"],
        file_content=None if content is None else base64.b64decode(content),
        metadata=row["metadata"],
    )


def extract_content(call: ExtractContentCall) -> Json:
    content = HANDLER.extract_skill_content(build_skill(call["skill"]))
    return {"content": content}


def extract_files(call: ExtractFilesCall) -> Json:
    files = HANDLER.extract_all_files(build_skill(call["skill"]))
    encoded: dict[str, Json] = {
        name: base64.b64encode(data).decode("ascii") for name, data in files.items()
    }
    return {"files": encoded}


def convert_tool(call: ConvertToolCall) -> Json:
    tool = ANSWER.validate_python(CONVERT(build_skill(call["skill"])))
    return {"tool": tool}


def convert_anthropic_tool(call: ConvertAnthropicCall) -> Json:
    tool = ANSWER.validate_python(CONVERT_ANTHROPIC(build_skill(call["skill"])))
    return {"tool": tool}


def inject(call: InjectCall) -> Json:
    data = dict(call["data"])
    INJECTOR.inject_skill_content_to_messages(
        data, call["contents"], use_anthropic_format=call["use_anthropic_format"]
    )
    return {"data": data}


def execution_tool() -> Json:
    tool = ANSWER.validate_python(get_litellm_code_execution_tool())
    return {"tool": tool}


def execution_tool_anthropic() -> Json:
    tool = ANSWER.validate_python(get_litellm_code_execution_tool_anthropic())
    return {"tool": tool}


def answer(call: Input) -> Json:
    result: Json
    if call["function"] == "extract_content":
        result = extract_content(call)
    elif call["function"] == "extract_files":
        result = extract_files(call)
    elif call["function"] == "convert_tool":
        result = convert_tool(call)
    elif call["function"] == "convert_anthropic_tool":
        result = convert_anthropic_tool(call)
    elif call["function"] == "inject":
        result = inject(call)
    elif call["function"] == "execution_tool":
        result = execution_tool()
    else:
        result = execution_tool_anthropic()
    return result


def run(input_value: Input) -> Json:
    capture = Capture()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        result = answer(input_value)
    finally:
        logger.removeHandler(capture)
    return {"result": result, "warnings": capture.messages}
