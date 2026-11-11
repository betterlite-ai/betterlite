import copy
from collections.abc import Callable, Mapping
from typing import Final, Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm.utils
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.utils import PreProcessNonDefaultParams, ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Handler: TypeAlias = Callable[[Fields], Json]


class Utils(Protocol):
    _get_non_default_params: Callable[..., Json]
    get_non_default_params: Callable[..., Json]
    provider_rejectable_params: Callable[..., set[str]]
    filter_out_litellm_params: Callable[..., Json]
    get_non_default_completion_params: Callable[..., Json]
    get_non_default_transcription_params: Callable[..., Json]
    _remove_unsupported_params: Callable[..., Json]
    _apply_openai_param_overrides: Callable[..., Json]
    _bedrock_route_for_request: Callable[..., Json]
    pre_process_non_default_params: Callable[..., Json]
    remove_sensitive_keys_from_dict: Callable[..., Json]
    pre_process_optional_params: Callable[..., Json]
    add_provider_specific_params_to_optional_params: Callable[..., Json]
    _remove_additional_properties: Callable[..., Json]
    _remove_strict_from_schema: Callable[..., Json]
    peek_reasoning_summary_aliases: Callable[..., Json]
    strip_reasoning_summary_aliases_from_optional_params: Callable[..., tuple[Fields, Json]]
    get_optional_params_transcription: Callable[..., Json]
    get_optional_params_image_gen: Callable[..., Json]
    _map_openai_size_to_vertex_ai_aspect_ratio: Callable[..., Json]


class Embedding(Protocol):
    embedding_pre_process_non_default_params: Callable[..., Json]


_UTILS: Final = cast(Utils, litellm.utils)
_EMBEDDING: Final = cast(Embedding, PreProcessNonDefaultParams)


class FixtureError(Exception):
    def __init__(self, status: int) -> None:
        super().__init__(f"fixture mapper failed with {status}")
        self.status = status


def _object(fields: Fields, name: str) -> Fields:
    value = fields.get(name, {})
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _text(fields: Fields, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_text(fields: Fields, name: str) -> str | None:
    if fields.get(name) is None:
        return None
    return _text(fields, name)


def _bool(fields: Fields, name: str) -> bool:
    value = fields.get(name, False)
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def _optional_bool(fields: Fields, name: str) -> bool | None:
    if fields.get(name) is None:
        return None
    return _bool(fields, name)


def _optional_int(fields: Fields, name: str) -> int | None:
    value = fields.get(name)
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def _strings(fields: Fields, name: str) -> list[str]:
    value = fields.get(name, [])
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list of strings")
    names: Final[list[str]] = []
    for item in value:
        if not isinstance(item, str):
            raise TypeError(f"{name} must be a list of strings")
        names.append(item)
    return names


def _optional_strings(fields: Fields, name: str) -> list[str] | None:
    if fields.get(name) is None:
        return None
    return _strings(fields, name)


class Mapper:
    def __init__(self, spec: Fields) -> None:
        self.supported = _strings(spec, "supported")
        self.fail_status = _optional_int(spec, "fail_status")
        self.records = _bool(spec, "records")

    def get_supported_openai_params(self, model: str = "") -> list[str]:
        del model
        return list(self.supported)

    def map_openai_params(
        self,
        non_default_params: Fields,
        optional_params: Fields,
        *,
        model: str = "",
        drop_params: bool = False,
    ) -> Fields:
        if self.fail_status is not None:
            raise FixtureError(self.fail_status)
        mapped = dict(optional_params)
        mapped.update(non_default_params)
        if self.records:
            mapped["seen_drop"] = drop_params
            mapped["seen_model"] = model
        return mapped

    get_supported_openai_params_stt = get_supported_openai_params
    map_openai_params_stt = map_openai_params


class Provide:
    def __init__(self, mapper: Mapper | None) -> None:
        self.mapper = mapper

    def __call__(self, *_args: Json, **_kwargs: Json) -> Mapper | None:
        return self.mapper


class RouteRecorder:
    def __init__(self) -> None:
        self.answer: Fields = {}

    def __call__(self, model: str, request_params: Mapping[str, Json]) -> None:
        self.answer = {"model": model, "params": dict(request_params)}


def _mapper(fields: Fields, name: str) -> Mapper | None:
    if fields.get(name) is None:
        return None
    return Mapper(_object(fields, name))


def _select_non_default(fields: Fields) -> Json:
    default_params: Final = dict.fromkeys(_strings(fields, "default_names"))
    return _UTILS._get_non_default_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        passed_params=_object(fields, "passed_params"),
        default_params=default_params,
        additional_drop_params=_optional_strings(fields, "additional_drop_params"),
    )


def _get_non_default_params(fields: Fields) -> Json:
    return _UTILS.get_non_default_params(passed_params=_object(fields, "passed_params"))


def _provider_rejectable(fields: Fields) -> Json:
    rejectable: Final = _UTILS.provider_rejectable_params(_object(fields, "passed_params"))
    return list[Json](sorted(rejectable))


def _filter_out_litellm(fields: Fields) -> Json:
    return _UTILS.filter_out_litellm_params(
        kwargs=_object(fields, "kwargs"),
        excluding=frozenset(_strings(fields, "excluding")),
    )


def _endpoint_kwargs(fields: Fields) -> Json:
    kwargs: Final = _object(fields, "kwargs")
    endpoint: Final = _text(fields, "endpoint")
    if endpoint == "completion":
        return _UTILS.get_non_default_completion_params(kwargs=kwargs)
    if endpoint == "transcription":
        return _UTILS.get_non_default_transcription_params(kwargs=kwargs)
    raise ValueError(f"unknown endpoint: {endpoint}")


def _remove_unsupported(fields: Fields) -> Json:
    return _UTILS._remove_unsupported_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        non_default_params=dict(_object(fields, "non_default_params")),
        supported_openai_params=_optional_strings(fields, "supported"),
    )


def _apply_overrides(fields: Fields) -> Json:
    non_default: Final = dict(_object(fields, "non_default_params"))
    optional: Final = _UTILS._apply_openai_param_overrides(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        optional_params=dict(_object(fields, "optional_params")),
        non_default_params=non_default,
        allowed_openai_params=_strings(fields, "allowed"),
    )
    return {"optional_params": optional, "non_default_params": non_default}


def _bedrock_route(fields: Fields) -> Json:
    recorder: Final = RouteRecorder()
    with patch.object(BedrockModelInfo, "get_bedrock_route", recorder):
        _UTILS._bedrock_route_for_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            model=_text(fields, "model"),
            passed_params=_object(fields, "passed_params"),
            additional_drop_params=_optional_strings(fields, "additional_drop_params"),
        )
    return recorder.answer


def _pre_process(fields: Fields) -> Json:
    request: Final = _object(fields, "request")
    return _UTILS.pre_process_non_default_params(
        passed_params=dict(_object(request, "passed_params")),
        special_params=_object(request, "special_params"),
        custom_llm_provider=_text(request, "custom_llm_provider"),
        additional_drop_params=_optional_strings(request, "additional_drop_params"),
        model="model",
        remove_sensitive_keys=_bool(fields, "remove_sensitive_keys"),
        add_provider_specific_params=_bool(fields, "add_provider_specific_params"),
    )


def _embedding_pre_process(fields: Fields) -> Json:
    request: Final = _object(fields, "request")
    return _EMBEDDING.embedding_pre_process_non_default_params(
        passed_params=dict(_object(request, "passed_params")),
        special_params=_object(request, "special_params"),
        custom_llm_provider=_text(request, "custom_llm_provider"),
        additional_drop_params=_optional_strings(request, "additional_drop_params"),
        model="model",
    )


def _remove_sensitive_keys(fields: Fields) -> Json:
    return _UTILS.remove_sensitive_keys_from_dict(d=dict(_object(fields, "params")))


def _function_fallback(fields: Fields) -> Json:
    non_default: Final = dict(_object(fields, "non_default_params"))
    optional: Final = _UTILS.pre_process_optional_params(
        passed_params=_object(fields, "passed_params"),
        non_default_params=non_default,
        custom_llm_provider=_text(fields, "custom_llm_provider"),
    )
    return {
        "optional_params": optional,
        "non_default_params": non_default,
        "add_function_to_prompt": litellm.add_function_to_prompt,
    }


def _add_provider_specific(fields: Fields) -> Json:
    return _UTILS.add_provider_specific_params_to_optional_params(
        optional_params=dict(_object(fields, "optional_params")),
        passed_params=dict(_object(fields, "passed_params")),
        custom_llm_provider=_text(fields, "custom_llm_provider"),
        openai_params=_strings(fields, "openai_params"),
        additional_drop_params=_optional_strings(fields, "additional_drop_params"),
    )


def _remove_additional_properties(fields: Fields) -> Json:
    schema: Final = copy.deepcopy(fields.get("schema"))
    return _UTILS._remove_additional_properties(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        schema=schema,
    )


def _remove_strict(fields: Fields) -> Json:
    schema: Final = copy.deepcopy(fields.get("schema"))
    return _UTILS._remove_strict_from_schema(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        schema=schema,
    )


def _peek_aliases(fields: Fields) -> Json:
    return _UTILS.peek_reasoning_summary_aliases(
        optional_params=_object(fields, "optional_params"),
    )


def _strip_aliases(fields: Fields) -> Json:
    stripped, summary = _UTILS.strip_reasoning_summary_aliases_from_optional_params(
        _object(fields, "optional_params")
    )
    return {"params": stripped, "summary": summary}


def _transcription(fields: Fields) -> Json:
    mapper: Final = _mapper(fields, "mapper")
    with (
        patch.object(
            ProviderConfigManager, "get_provider_audio_transcription_config", Provide(mapper)
        ),
        patch.object(litellm, "GroqSTTConfig", Provide(mapper)),
    ):
        return _UTILS.get_optional_params_transcription(
            **{
                **_object(fields, "params"),
                "model": _text(fields, "model"),
                "custom_llm_provider": _text(fields, "custom_llm_provider"),
                "drop_params": _optional_bool(fields, "drop_params"),
            },
        )


def _image_generation(fields: Fields) -> Json:
    bedrock: Final = Provide(_mapper(fields, "bedrock"))
    with patch.object(BedrockImageGeneration, "get_config_class", bedrock):
        return _UTILS.get_optional_params_image_gen(
            **{
                **_object(fields, "params"),
                "model": _optional_text(fields, "model"),
                "custom_llm_provider": _optional_text(fields, "custom_llm_provider"),
                "additional_drop_params": _optional_strings(fields, "additional_drop_params"),
                "provider_config": _mapper(fields, "provider_config"),
                "drop_params": _optional_bool(fields, "drop_params"),
            },
        )


def _vertex_aspect_ratio(fields: Fields) -> Json:
    return _UTILS._map_openai_size_to_vertex_ai_aspect_ratio(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        size=_optional_text(fields, "size"),
    )


_HANDLERS: Final[dict[str, Handler]] = {
    "select_non_default": _select_non_default,
    "get_non_default_params": _get_non_default_params,
    "provider_rejectable": _provider_rejectable,
    "filter_out_litellm": _filter_out_litellm,
    "endpoint_kwargs": _endpoint_kwargs,
    "remove_unsupported": _remove_unsupported,
    "apply_overrides": _apply_overrides,
    "bedrock_route": _bedrock_route,
    "pre_process": _pre_process,
    "embedding_pre_process": _embedding_pre_process,
    "remove_sensitive_keys": _remove_sensitive_keys,
    "function_fallback": _function_fallback,
    "add_provider_specific": _add_provider_specific,
    "remove_additional_properties": _remove_additional_properties,
    "remove_strict": _remove_strict,
    "peek_aliases": _peek_aliases,
    "strip_aliases": _strip_aliases,
    "transcription": _transcription,
    "image_generation": _image_generation,
    "vertex_aspect_ratio": _vertex_aspect_ratio,
}


def _answer(call: Fields) -> Json:
    handler: Final = _HANDLERS.get(_text(call, "function"))
    if handler is None:
        raise ValueError(f"unknown b5362 function: {call.get('function')}")
    litellm.drop_params = _bool(call, "litellm_drop_params")
    litellm.add_function_to_prompt = False
    try:
        return handler(call)
    except FixtureError as failure:
        return {"error": failure.status}
    except UnsupportedParamsError as failure:
        return {"error": failure.status_code}


def run(input_value: Fields) -> Json:
    calls: Final = input_value.get("calls")
    if not isinstance(calls, list):
        raise TypeError("calls must be a list")
    previous_drop: Final = litellm.drop_params
    previous_prompt: Final = litellm.add_function_to_prompt
    try:
        return [_answer(cast(Fields, call)) for call in calls]
    finally:
        litellm.drop_params = previous_drop
        litellm.add_function_to_prompt = previous_prompt
