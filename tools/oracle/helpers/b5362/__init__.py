from collections.abc import Callable, Mapping
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm import utils
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.base_llm import base_utils
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]


class MapperSpec(TypedDict):
    supported: list[str]
    fail_status: NotRequired[int | None]
    records: NotRequired[bool]
    rewrite_refs: NotRequired[bool]


class RequestSpec(TypedDict):
    passed_params: Fields
    special_params: Fields
    custom_llm_provider: str
    additional_drop_params: NotRequired[list[str] | None]


class Call(TypedDict):
    function: str
    passed_params: Fields
    default_names: list[str]
    additional_drop_params: NotRequired[list[str] | None]
    kwargs: Fields
    excluding: list[str]
    endpoint: NotRequired[Literal["completion", "transcription"] | None]
    non_default_params: Fields
    optional_params: Fields
    model: str
    request: RequestSpec
    remove_sensitive_keys: NotRequired[bool | None]
    add_provider_specific_params: NotRequired[bool | None]
    params: Fields
    custom_llm_provider: str
    openai_params: list[str]
    drop_params: NotRequired[bool | None]
    litellm_drop_params: NotRequired[bool | None]
    mapper: NotRequired[MapperSpec | None]
    provider_config: NotRequired[MapperSpec | None]
    bedrock: NotRequired[MapperSpec | None]


class Input(TypedDict):
    calls: list[Call]


DEFAULTS: Call = {
    "function": "",
    "passed_params": {},
    "default_names": [],
    "kwargs": {},
    "excluding": [],
    "non_default_params": {},
    "optional_params": {},
    "params": {},
    "openai_params": [],
    "model": "",
    "custom_llm_provider": "",
    "request": {"passed_params": {}, "special_params": {}, "custom_llm_provider": ""},
}


class FixtureError(Exception):
    def __init__(self, status: int) -> None:
        super().__init__(f"fixture mapper failed with {status}")
        self.status = status


class Mapper:
    def __init__(self, spec: MapperSpec) -> None:
        self.spec = spec
        self.get_supported_openai_params = Provide(spec["supported"])
        self.get_supported_openai_params_stt = self.get_supported_openai_params
        self.map_openai_params_stt = self.map_openai_params
        self.get_json_schema_from_pydantic_object = self.schema_for

    def map_openai_params(
        self,
        non_default_params: Fields,
        optional_params: Fields,
        *,
        model: str = "",
        drop_params: bool = False,
    ) -> Fields:
        if (status := self.spec.get("fail_status")) is not None:
            raise FixtureError(status)
        mapped = {**optional_params, **non_default_params}
        if self.spec.get("records") is True:
            mapped["seen_drop"] = drop_params
            mapped["seen_model"] = model
        return mapped

    def schema_for(self, response_format: Fields) -> Json:
        template = "/$defs/{model}" if self.spec.get("rewrite_refs") is True else None
        return base_utils.type_to_response_format_param(response_format, ref_template=template)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L179


class Provide:
    def __init__(self, value: Mapper | list[str] | None) -> None:
        self.value = value

    def __call__(self, *_args: Json, **_kwargs: Json) -> Mapper | list[str] | None:
        return self.value


class RouteRecorder:
    def __init__(self) -> None:
        self.answer: Fields = {}

    def __call__(self, model: str, request_params: Mapping[str, Json]) -> None:
        self.answer = {"model": model, "params": dict(request_params)}


def build_mapper(spec: MapperSpec | None) -> Mapper | None:
    if spec is None:
        return None
    return Mapper(spec)


def bedrock_route(call: Call) -> Json:
    recorder = RouteRecorder()
    BedrockModelInfo.get_bedrock_route = recorder  # type: ignore[method-assign, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/common_utils.py#L1347
    utils._bedrock_route_for_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3489
        call["model"], call["passed_params"], call.get("additional_drop_params")
    )
    return recorder.answer


def pre_process(call: Call) -> Json:
    request = call["request"]
    return utils.pre_process_non_default_params(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4281
        passed_params=dict(request["passed_params"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4282
        special_params=request["special_params"],
        custom_llm_provider=request["custom_llm_provider"],
        additional_drop_params=request.get("additional_drop_params"),
        model="model",
        remove_sensitive_keys=call.get("remove_sensitive_keys") is True,
        add_provider_specific_params=call.get("add_provider_specific_params") is True,
        provider_config=build_mapper(call.get("provider_config")),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4289
    )


def function_fallback(call: Call) -> Json:
    remaining = dict(call["non_default_params"])
    return {
        "optional_params": utils.pre_process_optional_params(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4356
            passed_params=call["passed_params"],
            non_default_params=remaining,
            custom_llm_provider=call["custom_llm_provider"],
        ),
        "non_default_params": remaining,
        "add_function_to_prompt": litellm.add_function_to_prompt,
    }


def strip_aliases(call: Call) -> Json:
    optional = call["optional_params"]
    stripped, summary = utils.strip_reasoning_summary_aliases_from_optional_params(optional)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10261
    return {"params": stripped, "summary": summary}  # type: ignore[misc, dict-item]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10261


def filter_out(call: Call) -> Json:
    endpoint = call.get("endpoint")
    if endpoint == "completion":
        return utils.get_non_default_completion_params(call["kwargs"])  # type: ignore[return-value]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10239
    if endpoint == "transcription":
        return utils.get_non_default_transcription_params(call["kwargs"])  # type: ignore[return-value]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10286
    return utils.filter_out_litellm_params(call["kwargs"], frozenset(call["excluding"]))  # type: ignore[return-value]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4196


def transcription(call: Call) -> Json:
    mapper = Provide(build_mapper(call.get("mapper")))
    ProviderConfigManager.get_provider_audio_transcription_config = mapper  # type: ignore[method-assign, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9013
    litellm.GroqSTTConfig = mapper  # type: ignore[misc, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1614
    options = {
        **call["params"],
        "model": call["model"],
        "custom_llm_provider": call["custom_llm_provider"],
        "drop_params": call.get("drop_params"),
    }
    return utils.get_optional_params_transcription(**options)  # type: ignore[arg-type, misc, no-any-return]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3510


def image_generation(call: Call) -> Json:
    BedrockImageGeneration.get_config_class = Provide(build_mapper(call.get("bedrock")))  # type: ignore[method-assign, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L66
    return utils.get_optional_params_image_gen(  # type: ignore[misc, no-any-return]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3617
        **call["params"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3617
        model=call.get("model"),
        custom_llm_provider=call.get("custom_llm_provider"),
        additional_drop_params=call.get("additional_drop_params"),
        provider_config=build_mapper(call.get("provider_config")),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3628
        drop_params=call.get("drop_params"),
    )


def select_non_default(call: Call) -> Json:
    defaults: dict[str, None] = dict.fromkeys(call["default_names"])
    return utils._get_non_default_params(  # noqa: SLF001  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3497
        passed_params=call["passed_params"],
        default_params=defaults,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3497
        additional_drop_params=call.get("additional_drop_params"),
    )


def add_provider_specific(call: Call) -> Json:
    optional = dict(call["optional_params"])
    passed = dict(call["passed_params"])
    provider = call["custom_llm_provider"]
    return utils.add_provider_specific_params_to_optional_params(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5049
        optional, passed, provider, call["openai_params"], call.get("additional_drop_params")
    )


HANDLERS: dict[str, Callable[[Call], Json]] = {
    "select_non_default": select_non_default,
    "filter_out_litellm": filter_out,
    "bedrock_route": bedrock_route,
    "pre_process": pre_process,
    "function_fallback": function_fallback,
    "add_provider_specific": add_provider_specific,
    "strip_aliases": strip_aliases,
    "transcription": transcription,
    "image_generation": image_generation,
}


def run(input_value: Input) -> Json:
    results: list[Json] = []
    route = BedrockModelInfo.get_bedrock_route
    transcribe = ProviderConfigManager.get_provider_audio_transcription_config
    groq = litellm.GroqSTTConfig  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1614
    images = BedrockImageGeneration.get_config_class
    for raw in input_value["calls"]:
        call: Call = {**DEFAULTS, **raw}
        litellm.drop_params = call.get("litellm_drop_params") is True
        litellm.add_function_to_prompt = False
        try:
            if call["function"] == "peek_aliases":
                results.append(utils.peek_reasoning_summary_aliases(call["optional_params"]))  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10243
            elif call["function"] == "remove_sensitive_keys":
                results.append(utils.remove_sensitive_keys_from_dict(dict(call["params"])))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L4342
            else:
                results.append(HANDLERS[call["function"]](call))
        except FixtureError as failure:
            results.append({"error": failure.status})
        except UnsupportedParamsError as failure:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
            results.append({"error": failure.status_code})
        finally:
            BedrockModelInfo.get_bedrock_route = route  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/common_utils.py#L1347
            ProviderConfigManager.get_provider_audio_transcription_config = transcribe  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9013
            litellm.GroqSTTConfig = groq  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1614
            BedrockImageGeneration.get_config_class = images  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L66
    return results
