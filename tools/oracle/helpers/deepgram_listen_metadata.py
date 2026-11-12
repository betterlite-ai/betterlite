from typing import Literal, TypeAlias, TypedDict

from litellm.llms.deepgram import common_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

Function: TypeAlias = Literal[
    "query_identity",
    "blank_model",
    "callbacks",
    "exact_streaming_price",
    "addon_flags",
    "channel_precedence",
    "duration_precedence",
    "final_transcript",
]


class Message(TypedDict, total=False):
    type: str
    channels: Json
    duration: Json
    start: Json
    channel_index: Json
    is_final: Json
    channel: Json


class Call(TypedDict):
    function: Function
    query: str
    api_base: str | None
    upstream_url: str
    messages: list[Message]


class Input(TypedDict):
    calls: list[Call]


def execute(call: Call) -> Json:
    function: str = call["function"]
    if function in {"query_identity", "blank_model"}:
        outcome: Json = common_utils.deepgram_listen_websocket_target(
            call["api_base"], call["query"]
        )
    elif function == "callbacks":
        names = common_utils.deepgram_listen_callback_params(call["query"])
        outcome = list(names)
    elif function == "exact_streaming_price":
        url = call["upstream_url"]
        answer: dict[str, Json] = {
            "registry_key": common_utils.deepgram_listen_registry_key(url),
            "priced": common_utils.deepgram_listen_is_priced(url),
        }
        outcome = answer
    elif function == "addon_flags":
        addons = common_utils.deepgram_listen_addon_pricing_models(call["upstream_url"])
        outcome = list(addons)
    elif function == "channel_precedence":
        outcome = common_utils.deepgram_listen_channel_count(call["messages"], call["upstream_url"])
    elif function == "duration_precedence":
        outcome = common_utils.deepgram_listen_audio_seconds(call["messages"])
    elif function == "final_transcript":
        outcome = common_utils.deepgram_listen_transcript(call["messages"])
    else:
        raise ValueError(f"unknown function {function}")
    return outcome


def run(input_value: Input) -> Json:
    calls = input_value["calls"]
    return [execute(call) for call in calls]
