from collections.abc import Callable, Mapping
from typing import Final, Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm.utils
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.base_llm import base_utils
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class Utils(Protocol):
    _bedrock_route_for_request: Callable[..., Json]
    _get_non_default_params: Callable[..., Json]
    add_provider_specific_params_to_optional_params: Callable[..., Json]
    filter_out_litellm_params: Callable[..., Json]
    get_non_default_completion_params: Callable[..., Json]
    get_non_default_transcription_params: Callable[..., Json]
    get_optional_params_image_gen: Callable[..., Json]
    get_optional_params_transcription: Callable[..., Json]
    peek_reasoning_summary_aliases: Callable[..., Json]
    pre_process_non_default_params: Callable[..., Json]
    pre_process_optional_params: Callable[..., Json]
    remove_sensitive_keys_from_dict: Callable[..., Json]
    strip_reasoning_summary_aliases_from_optional_params: Callable[..., tuple[Fields, Json]]


class BaseUtils(Protocol):
    type_to_response_format_param: Callable[..., Json]


_UTILS: Final = cast(Utils, litellm.utils)
_BASE_UTILS: Final = cast(BaseUtils, base_utils)


def _obj(fields: Fields, name: str) -> Fields:
    return cast(Fields, fields.get(name, {}))


def _str(fields: Fields, name: str) -> str:
    return cast(str, fields[name])


def _flag(fields: Fields, name: str) -> bool:
    return fields.get(name) is True


def _strs(fields: Fields, name: str) -> list[str]:
    return cast("list[str]", fields.get(name, []))


class FixtureError(Exception):
    def __init__(self, status: int) -> None:
        super().__init__(f"fixture mapper failed with {status}")
        self.status = status


class Mapper:
    def __init__(self, spec: Fields) -> None:
        self.supported: Final = _strs(spec, "supported")
        self.fail_status: Final = cast("int | None", spec.get("fail_status"))
        self.records: Final = _flag(spec, "records")
        self.rewrite_refs: Final = _flag(spec, "rewrite_refs")

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
        mapped: Final = {**optional_params, **non_default_params}
        if self.records:
            mapped["seen_drop"] = drop_params
            mapped["seen_model"] = model
        return mapped

    def get_json_schema_from_pydantic_object(self, response_format: Fields) -> Json:
        template: Final = "/$defs/{model}" if self.rewrite_refs else None
        return _BASE_UTILS.type_to_response_format_param(response_format, ref_template=template)

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
    return Mapper(_obj(fields, name))


def _bedrock_route(fields: Fields) -> Json:
    recorder: Final = RouteRecorder()
    with patch.object(BedrockModelInfo, "get_bedrock_route", recorder):
        _UTILS._bedrock_route_for_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            model=_str(fields, "model"),
            passed_params=_obj(fields, "passed_params"),
            additional_drop_params=fields.get("additional_drop_params"),
        )
    return recorder.answer


def _pre_process(fields: Fields) -> Json:
    request: Final = _obj(fields, "request")
    return _UTILS.pre_process_non_default_params(
        passed_params=dict(_obj(request, "passed_params")),
        special_params=_obj(request, "special_params"),
        custom_llm_provider=_str(request, "custom_llm_provider"),
        additional_drop_params=request.get("additional_drop_params"),
        model="model",
        remove_sensitive_keys=_flag(fields, "remove_sensitive_keys"),
        add_provider_specific_params=_flag(fields, "add_provider_specific_params"),
        provider_config=_mapper(fields, "provider_config"),
    )


def _function_fallback(fields: Fields) -> Json:
    non_default: Final = dict(_obj(fields, "non_default_params"))
    optional = _UTILS.pre_process_optional_params(
        passed_params=_obj(fields, "passed_params"),
        non_default_params=non_default,
        custom_llm_provider=_str(fields, "custom_llm_provider"),
    )
    return {
        "optional_params": optional,
        "non_default_params": non_default,
        "add_function_to_prompt": litellm.add_function_to_prompt,
    }


def _strip_aliases(fields: Fields) -> Json:
    stripped, summary = _UTILS.strip_reasoning_summary_aliases_from_optional_params(
        _obj(fields, "optional_params")
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
                **_obj(fields, "params"),
                "model": _str(fields, "model"),
                "custom_llm_provider": _str(fields, "custom_llm_provider"),
                "drop_params": fields.get("drop_params"),
            },
        )


def _image_generation(fields: Fields) -> Json:
    bedrock: Final = Provide(_mapper(fields, "bedrock"))
    with patch.object(BedrockImageGeneration, "get_config_class", bedrock):
        return _UTILS.get_optional_params_image_gen(
            **{
                **_obj(fields, "params"),
                "model": fields.get("model"),
                "custom_llm_provider": fields.get("custom_llm_provider"),
                "additional_drop_params": fields.get("additional_drop_params"),
                "provider_config": _mapper(fields, "provider_config"),
                "drop_params": fields.get("drop_params"),
            },
        )


_HANDLERS: Final[dict[str, Callable[[Fields], Json]]] = {
    "select_non_default": lambda fields: _UTILS._get_non_default_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        passed_params=_obj(fields, "passed_params"),
        default_params=dict.fromkeys(_strs(fields, "default_names")),
        additional_drop_params=fields.get("additional_drop_params"),
    ),
    "filter_out_litellm": lambda fields: _UTILS.filter_out_litellm_params(
        kwargs=_obj(fields, "kwargs"),
        excluding=frozenset(_strs(fields, "excluding")),
    ),
    "non_default_completion": lambda fields: _UTILS.get_non_default_completion_params(
        kwargs=_obj(fields, "kwargs")
    ),
    "non_default_transcription": lambda fields: _UTILS.get_non_default_transcription_params(
        kwargs=_obj(fields, "kwargs")
    ),
    "bedrock_route": _bedrock_route,
    "pre_process": _pre_process,
    "remove_sensitive_keys": lambda fields: _UTILS.remove_sensitive_keys_from_dict(
        d=dict(_obj(fields, "params"))
    ),
    "function_fallback": _function_fallback,
    "add_provider_specific": lambda fields: _UTILS.add_provider_specific_params_to_optional_params(
        optional_params=dict(_obj(fields, "optional_params")),
        passed_params=dict(_obj(fields, "passed_params")),
        custom_llm_provider=_str(fields, "custom_llm_provider"),
        openai_params=_strs(fields, "openai_params"),
        additional_drop_params=fields.get("additional_drop_params"),
    ),
    "peek_aliases": lambda fields: _UTILS.peek_reasoning_summary_aliases(
        optional_params=_obj(fields, "optional_params")
    ),
    "strip_aliases": _strip_aliases,
    "transcription": _transcription,
    "image_generation": _image_generation,
}


def _answer(call: Fields) -> Json:
    handler: Final = _HANDLERS.get(_str(call, "function"))
    if handler is None:
        raise ValueError(f"unknown b5362 function: {call.get('function')}")
    litellm.drop_params = _flag(call, "litellm_drop_params")
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
