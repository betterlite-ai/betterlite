import asyncio
import io
from collections.abc import Callable
from contextlib import redirect_stdout
from functools import partial
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.openai.chat.gpt_5_transformation import (
    OpenAIGPT5Config as Gpt5Config,
)
from litellm.llms.openai.chat.gpt_5_transformation import (
    is_gpt_reasoning_series_name,
)
from litellm.llms.openai.chat.gpt_audio_transformation import OpenAIGPTAudioConfig as AudioConfig
from litellm.llms.openai.chat.o_series_transformation import OpenAIOSeriesConfig as OSeriesConfig
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

litellm.suppress_debug_info = True


class Resolved(TypedDict):
    model: str
    provider: str


class Message(TypedDict):
    role: str
    content: str
    name: NotRequired[str]


class Probe(TypedDict):
    model: str
    chat_models: NotRequired[list[str]]
    text_models: NotRequired[list[str]]
    model_cost: NotRequired[dict[str, Json]]
    requested: NotRequired[Object]
    existing: NotRequired[Object]
    drop_params: NotRequired[bool]
    global_drop_params: NotRequired[bool]
    resolved: NotRequired[Resolved | None]
    effective_effort: NotRequired[str | None]
    messages: NotRequired[list[Message]]


class Probes(TypedDict):
    probes: list[Probe]


Handler: TypeAlias = Callable[[Probe], Json]


def install(probe: Probe) -> None:
    given_cost = probe.get("model_cost")
    costs: dict[str, Json] = {} if given_cost is None else given_cost
    litellm.model_cost = costs
    given_chat = probe.get("chat_models")
    chat: set[str] = set() if given_chat is None else set(given_chat)
    litellm.open_ai_chat_completion_models = chat
    given_text = probe.get("text_models")
    text: set[str] = set() if given_text is None else set(given_text)
    litellm.open_ai_text_completion_models = text
    global_drop = probe.get("global_drop_params")
    litellm.drop_params = False if global_drop is None else global_drop


def maps_of(probe: Probe) -> tuple[Object, Object]:
    requested = probe.get("requested")
    existing = probe.get("existing")
    non_default: Object = {} if requested is None else dict(requested)
    optional: Object = {} if existing is None else dict(existing)
    return non_default, optional


def drop_of(probe: Probe) -> bool:
    value = probe.get("drop_params")
    return False if value is None else value


def mapping_value(probe: Probe, *, series: Literal["gpt", "o_series"]) -> Object:
    install(probe)
    config = OSeriesConfig() if series == "o_series" else Gpt5Config()
    non_default, optional = maps_of(probe)
    try:
        config.map_openai_params(non_default, optional, probe["model"], drop_of(probe))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_5_transformation.py#L255
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": error.status_code}
    return {"non_default_params": non_default, "optional_params": optional}


def gpt_supported_list(probe: Probe) -> list[Json]:
    model = probe["model"]
    config = Gpt5Config()
    return cast("list[Json]", config.get_supported_openai_params(model=model))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_5_transformation.py#L212


def search_mapping_value(probe: Probe) -> Object:
    install(probe)
    params = gpt_supported_list(probe)
    config = Gpt5Config()
    non_default, optional = maps_of(probe)
    try:
        config.map_openai_params(non_default, optional, probe["model"], drop_of(probe))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_5_transformation.py#L255
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        rejected: Object = {"supported": params, "error": error.status_code}
        return rejected
    mapped: Object = {
        "supported": params,
        "non_default_params": non_default,
        "optional_params": optional,
    }
    return mapped


def o_series_supported_value(probe: Probe) -> Object:
    install(probe)
    model = probe["model"]
    config = OSeriesConfig()
    params = cast("list[Json]", config.get_supported_openai_params(model=model))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/o_series_transformation.py#L46
    result: Object = {"supported": params}
    return result


def gpt_supported_value(probe: Probe) -> Object:
    install(probe)
    result: Object = {"supported": gpt_supported_list(probe)}
    return result


def audio_value(probe: Probe) -> Object:
    install(probe)
    model = probe["model"]
    config = AudioConfig()
    params = cast("list[Json]", config.get_supported_openai_params(model=model))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_audio_transformation.py#L23
    non_default, optional = maps_of(probe)
    config.map_openai_params(non_default, optional, probe["model"], drop_of(probe))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_5_transformation.py#L255
    result: Object = {
        "supported": params,
        "non_default_params": non_default,
        "optional_params": optional,
    }
    return result


def policy_of(model: str) -> str:
    series = OSeriesConfig()
    gpt5 = Gpt5Config()
    audio = AudioConfig()
    if series.is_model_o_series_model(model=model):
        return "o_series"
    if is_gpt_reasoning_series_name(model):
        return "gpt5_search" if gpt5.is_model_gpt_5_search_model(model=model) else "gpt5"
    if audio.is_model_gpt_audio_model(model=model):
        return "audio"
    return "base"


def policy_value(probe: Probe) -> Object:
    install(probe)
    result: Object = {"policy": policy_of(probe["model"])}
    return result


def version_pair(version: tuple[int, int] | None) -> Json:
    if version is None:
        return None
    return [version[0], version[1]]


def version_flags(model: str, config: Gpt5Config) -> Object:
    return {
        "gpt_5_4_plus": config.is_model_gpt_5_4_plus_model(model=model),
        "gpt_5_6_plus": config.is_model_gpt_5_6_plus_model(model=model),
        "gpt_6_plus": config.is_model_gpt_6_plus_model(model=model),
        "gpt_5_2": config.is_model_gpt_5_2_model(model=model),
    }


def version_value(probe: Probe) -> Object:
    model = probe["model"]
    version = Gpt5Config()._gpt_series_version(model)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_5_transformation.py#L119
    flags = version_flags(model, Gpt5Config())
    flags["version"] = version_pair(version)
    return flags


def gpt6_value(probe: Probe) -> Object:
    install(probe)
    value = version_value(probe)
    value["policy"] = policy_of(probe["model"])
    return value


def effort_value(probe: Probe) -> Object:
    install(probe)
    config = Gpt5Config()
    resolve = config.effort_resolves_to_none(
        model=probe["model"], effective_effort=probe.get("effective_effort")
    )
    return {"resolves_to_none": resolve}


def messages_value(probe: Probe) -> Object:
    install(probe)
    config = OSeriesConfig()
    model = probe["model"]
    given = probe.get("messages")
    messages = [] if given is None else given
    typed = cast("list[AllMessageValues]", messages)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    empty: Object = {}
    bodies: dict[str, Json] = {}
    sync_body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L476
        "dict[str, Json]",
        config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L476
            model=model,
            messages=typed,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=empty,
            litellm_params=empty,
            headers=empty,
        ),
    )
    async_body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L513
        "dict[str, Json]",
        asyncio.run(
            config.async_transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L513  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L513
                model=model,
                messages=typed,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
                optional_params=empty,
                litellm_params=empty,
                headers=empty,
            )
        ),
    )
    bodies["sync"] = sync_body["messages"]
    bodies["async"] = async_body["messages"]
    return bodies


HANDLERS: dict[str, Handler] = {
    "gpt5_search_allowlist": search_mapping_value,
    "gpt5_chat_exclusion": policy_value,
    "gpt6_family_selection": gpt6_value,
    "dict_effort_normalization": partial(mapping_value, series="gpt"),
    "xhigh_opt_in": partial(mapping_value, series="gpt"),
    "low_minimal_opt_out": partial(mapping_value, series="gpt"),
    "catalog_before_default_key": effort_value,
    "temperature_while_thinking": partial(mapping_value, series="gpt"),
    "sampling_with_none": partial(mapping_value, series="gpt"),
    "o1_developer_roles": messages_value,
    "audio_token_parameter": audio_value,
    "gpt5_regular_allowlist": gpt_supported_value,
    "version_boundaries": version_value,
    "o_series_capability_fallback": o_series_supported_value,
    "o_series_temperature_limit": partial(mapping_value, series="o_series"),
}


def run(input_value: dict[str, Probes]) -> Json:
    if len(input_value) != 1:
        raise ValueError("expected one openai_reasoning_models case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_reasoning_models case: {name}")
    with redirect_stdout(io.StringIO()):
        return [handler(probe) for probe in input_value[name]["probes"]]
