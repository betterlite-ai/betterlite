from collections.abc import Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

import litellm
from litellm.llms.fireworks_ai.chat.transformation import FireworksAIConfig
from pydantic import JsonValue, TypeAdapter


class CatalogModel(TypedDict):
    key: str
    model_info: dict[str, JsonValue]


class SettingsSnapshot(TypedDict, total=False):
    tools: list[JsonValue] | None
    tool_choice: str | dict[str, JsonValue] | None
    max_tokens: int | None
    temperature: int | None
    top_p: int | None
    top_k: int | None
    frequency_penalty: int | None
    presence_penalty: int | None
    n: int | None
    stop: str | list[JsonValue] | None
    response_format: dict[str, JsonValue] | None
    user: str | None
    logprobs: int | None
    reasoning_effort: str | None
    prompt_truncate_len: int | None
    # ast-grep-ignore: no-long-names
    context_length_exceeded_behavior: Literal["error", "truncate"] | None


class FireworksConfig(Protocol):
    def get_config(self) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def map_extra_body_params(
        self, optional_params: Mapping[str, JsonValue], model: str
    ) -> dict[str, JsonValue]: ...


class SupportedStep(TypedDict):
    op: Literal["supported_params"]
    model: str


class MapStep(TypedDict):
    op: Literal["map_params"]
    model: str
    non_default_params: NotRequired[dict[str, JsonValue]]
    ordered_params: NotRequired[list[tuple[str, JsonValue]]]
    optional_params: NotRequired[dict[str, JsonValue]]


class ExtraBodyStep(TypedDict):
    op: Literal["map_extra_body"]
    model: str
    optional_params: NotRequired[dict[str, JsonValue]]


class ConfigStep(TypedDict):
    op: Literal["config"]
    settings: NotRequired[dict[str, JsonValue]]


Step = SupportedStep | MapStep | ExtraBodyStep | ConfigStep


class Input(TypedDict):
    catalog: NotRequired[list[CatalogModel]]
    steps: list[Step]


def fresh_config(settings: dict[str, JsonValue]) -> FireworksConfig:
    snapshot = TypeAdapter(SettingsSnapshot).validate_python(settings)
    return FireworksAIConfig(**snapshot)


def model_cost_of(data: Input) -> dict[str, dict[str, JsonValue]]:
    entries = data.get("catalog")
    if entries is None:
        return {}
    return {entry["key"]: entry["model_info"] for entry in entries}


def supported_value(step: SupportedStep) -> JsonValue:
    supported = fresh_config({}).get_supported_openai_params(step["model"])
    return [*supported]


def request_params(step: MapStep) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    given = step.get("non_default_params")
    if given is not None:
        params.update(given)
    ordered = step.get("ordered_params")
    if ordered is not None:
        params.update(ordered)
    return params


def optional_params_of(step: MapStep | ExtraBodyStep) -> dict[str, JsonValue]:
    optional: dict[str, JsonValue] = {}
    given = step.get("optional_params")
    if given is not None:
        optional.update(given)
    return optional


def map_value(step: MapStep) -> JsonValue:
    config = fresh_config({})
    return config.map_openai_params(
        request_params(step), optional_params_of(step), step["model"], drop_params=False
    )


def extra_body_value(step: ExtraBodyStep) -> JsonValue:
    config = fresh_config({})
    return config.map_extra_body_params(optional_params_of(step), step["model"])


def config_value(step: ConfigStep) -> JsonValue:
    settings: dict[str, JsonValue] = {}
    given = step.get("settings")
    if given is not None:
        settings.update(given)
    return fresh_config(settings).get_config()


def run_step(step: Step) -> JsonValue:
    if step["op"] == "supported_params":
        return supported_value(step)
    if step["op"] == "map_params":
        return map_value(step)
    if step["op"] == "map_extra_body":
        return extra_body_value(step)
    return config_value(step)


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = model_cost_of(input_value)
    outcomes: list[JsonValue] = []
    outcomes.extend(run_step(step) for step in input_value["steps"])
    return outcomes
