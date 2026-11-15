from typing import Literal, Required, TypedDict

import litellm
from litellm.proxy.guardrails import init_guardrails
from litellm.types.guardrails import Role
from pydantic import JsonValue, TypeAdapter

CallbackArgs = dict[str, dict[str, JsonValue]]


class Spec(TypedDict, total=False):
    callbacks: Required[list[str]]
    default_on: bool
    logging_only: bool | None
    enabled_roles: list[Role] | None
    callback_args: CallbackArgs


class Input(TypedDict):
    function: Literal["legacy_plan"]
    guardrails: list[dict[str, Spec]]
    started: list[str]


class ItemOutput(TypedDict):
    callbacks: list[str]
    default_on: bool
    logging_only: bool | None
    enabled_roles: list[str] | None
    callback_args: CallbackArgs


class Output(TypedDict):
    callbacks: list[str]
    callback_args: CallbackArgs | None
    by_name: dict[str, ItemOutput]


class Recorder:
    def __init__(self) -> None:
        self.callbacks: list[str] = []
        self.callback_args: CallbackArgs | None = None

    def record(
        self,
        value: list[str],
        premium_user: bool,
        config_file_path: str,
        litellm_settings: dict[str, JsonValue],
        callback_specific_params: CallbackArgs | None = None,
    ) -> None:
        # LiteLLM builds the list from a set, so only the sorted list is comparable
        self.callbacks = sorted(value)
        self.callback_args = callback_specific_params


def run(input_value: Input) -> Output:
    recorder = Recorder()
    settings: dict[str, JsonValue] = {}
    init_guardrails.all_guardrails.clear()
    litellm.guardrail_name_config_map.clear()
    litellm.callbacks = [*input_value["started"]]
    init_guardrails.initialize_callbacks_on_proxy = recorder.record
    name_map = init_guardrails.initialize_guardrails(
        input_value["guardrails"],
        premium_user=True,
        config_file_path="",
        litellm_settings=settings,
    )
    by_name = {
        name: TypeAdapter(ItemOutput).validate_json(item.model_dump_json())
        for name, item in name_map.items()
    }
    return {
        "callbacks": recorder.callbacks,
        "callback_args": recorder.callback_args,
        "by_name": by_name,
    }
