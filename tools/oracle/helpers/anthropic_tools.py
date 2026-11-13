from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from openai import BadRequestError
from pydantic import JsonValue

# a model without supports_forced_tool_use, so the mapped choice comes back unchanged
MODEL = "claude-sonnet-4-5"


class Config(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


# LiteLLM leaves the optional params bare, so this typed view is the public entry to the tools
CONFIG: Config = AnthropicConfig()


class ToolsCall(TypedDict):
    function: Literal["tools"]
    tools: list[JsonValue]


class ChoiceCall(TypedDict):
    function: Literal["tool_choice"]
    tool_choice: JsonValue
    parallel_tool_calls: bool | None


Call: TypeAlias = ToolsCall | ChoiceCall


class Input(TypedDict):
    calls: list[Call]


class Mapped(TypedDict):
    ok: bool
    tools: JsonValue
    mcp_servers: JsonValue


class Chosen(TypedDict):
    ok: bool
    tool_choice: JsonValue


class Failure(TypedDict):
    ok: bool
    status: int | None


Answer: TypeAlias = Mapped | Chosen | Failure


class Output(TypedDict):
    calls: list[Answer]


def mapped_params(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    optional: dict[str, JsonValue] = {}
    return CONFIG.map_openai_params(
        non_default_params=params,
        optional_params=optional,
        model=MODEL,
        drop_params=False,
    )


def tools_answer(call: ToolsCall) -> Mapped:
    mapped = mapped_params({"tools": call["tools"]})
    return {
        "ok": True,
        "tools": mapped.get("tools"),
        "mcp_servers": mapped.get("mcp_servers"),
    }


def choice_answer(call: ChoiceCall) -> Chosen:
    mapped = mapped_params(
        {
            "tool_choice": call["tool_choice"],
            "parallel_tool_calls": call["parallel_tool_calls"],
        }
    )
    return {"ok": True, "tool_choice": mapped.get("tool_choice")}


def answer(call: Call) -> Answer:
    try:
        if call["function"] == "tools":
            return tools_answer(call)
        return choice_answer(call)
    except BadRequestError as error:
        return {"ok": False, "status": error.status_code}
    except (KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}


def run(input_value: Input) -> Output:
    answers: list[Answer] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
