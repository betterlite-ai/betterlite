import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, TypeAlias, cast

from litellm.integrations.anthropic_cache_control_hook import CARRY_UNMATCHED_MESSAGE_POINTS
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai_like.responses.transformation import OpenAILikeResponsesConfig
from litellm.responses import main as selection
from litellm.types.llms.openai import AllMessageValues
from litellm.types.prompts.init_prompts import PromptSpec
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ASYNC_KEY = "_async_prompt_merged_params"


class SelectionCase(TypedDict):
    action: Literal["selection"]
    provider: str
    model: str
    use_chat_completions_api: bool
    model_info: JsonValue
    kwargs: dict[str, JsonValue]
    allowed_openai_params: list[str]


class PrefixesCase(TypedDict):
    action: Literal["prefixes"]
    model: str
    flag_present: bool
    flag: NotRequired[JsonValue]


class CompatibleCase(TypedDict):
    action: Literal["compatible"]
    environment: dict[str, str]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    headers: dict[str, str]


class CredentialCase(TypedDict):
    action: Literal["credential_boundary"]
    original_model: str
    swapped_model: str
    provider: NotRequired[str | None]
    prompt_id: NotRequired[str | None]
    kwargs: dict[str, JsonValue]
    model_info: NotRequired[JsonValue]


class HookCase(TypedDict):
    mode: Literal["answer", "skip", "throw"]
    model: str
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class AsyncOutcomeCase(TypedDict):
    merged_optional_params: dict[str, JsonValue]
    deployment_model_info: JsonValue


class PromptCallCase(TypedDict):
    action: Literal["prompt_call"]
    input: JsonValue
    model: str
    provider: NotRequired[str | None]
    use_chat_completions_api: bool
    logging: bool
    hook: NotRequired[HookCase | None]
    async_outcome: NotRequired[AsyncOutcomeCase | None]
    kwargs: dict[str, JsonValue]


class FileMappingCase(TypedDict):
    action: Literal["file_mapping"]
    input: JsonValue
    tools: NotRequired[list[JsonValue] | None]
    model_info: NotRequired[JsonValue]
    mapping: NotRequired[JsonValue]


Input: TypeAlias = (
    SelectionCase | PrefixesCase | CompatibleCase | CredentialCase | PromptCallCase | FileMappingCase
)

PromptApply: TypeAlias = Callable[
    [JsonValue, str, str | None, Logging | None, dict[str, object], dict[str, object], bool],
    tuple[JsonValue, str, str | None, JsonValue],
]
FilesApply: TypeAlias = Callable[
    [JsonValue, JsonValue, dict[str, object], dict[str, object]], tuple[JsonValue, JsonValue]
]


class ScriptedLogging(Logging):
    hook: HookCase | None
    requests: list[JsonValue]
    calls: list[JsonValue]

    def __init__(self, hook: HookCase | None) -> None:
        self.hook = hook
        self.requests = []
        self.calls = []

    @override
    def should_run_prompt_management_hooks(  # ast-grep-ignore: no-long-names
        self,
        non_default_params: dict[str, object],
        prompt_id: str | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
    ) -> bool:
        del tools
        request: dict[str, object] = {"prompt_id": prompt_id, "param_count": len(non_default_params)}
        self.requests.append(JSON.validate_python(request))
        hook = self.hook
        return hook is not None and hook["mode"] != "skip"

    @override
    def get_chat_completion_prompt(
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, object],
        prompt_variables: dict[str, JsonValue] | None,
        prompt_id: str | None = None,
        prompt_spec: PromptSpec | None = None,
        prompt_management_logger: CustomLogger | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        request_kwargs: dict[str, object] | None = None,
        injected_for_every_deployment: bool = False,
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        del non_default_params, prompt_spec, prompt_management_logger, injected_for_every_deployment
        signal = None if request_kwargs is None else request_kwargs.get(CARRY_UNMATCHED_MESSAGE_POINTS)
        call: dict[str, object] = {
            "model": model,
            "messages": JSON.validate_python(messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
            "prompt_id": prompt_id,
            "prompt_label": prompt_label,
            "prompt_version": prompt_version,
            "prompt_variables": prompt_variables,
            "signal": signal,
        }
        self.calls.append(JSON.validate_python(call))
        hook = self.hook
        if hook is None or hook["mode"] == "skip":
            raise ValueError("the hook must not run")
        if hook["mode"] == "throw":
            raise ValueError("boom")
        return hook["model"], hook["messages"], dict(hook["optional_params"])


CONFIG: Final = OpenAILikeResponsesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L17-L51

RESOLVE_CONFIG: Final = selection._resolve_responses_api_provider_config  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L510-L519
BRIDGES: Final = selection._bridges_to_chat_completions  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L427-L431
ENCRYPTED_FAILURE: Final = selection._encrypted_task_support_failure  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L461-L471
PASSES_THROUGH: Final = selection._deployment_passes_through_responses  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L489-L495
API_BASE: Final = selection._api_base_kwarg  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L521-L524
BRIDGE_KWARGS: Final = selection._bridge_kwargs  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L437-L458
WILL_BRIDGE: Final = selection._will_bridge_to_chat_completions  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L526-L545
NORMALIZE: Final = selection._normalize_openai_chat_completions_responses_model  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L881-L892
POP_FLAG: Final = selection._pop_use_chat_completions_api_kw  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L894-L897
STRIP_PREFIX: Final = selection._strip_responses_routing_prefix  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L903-L906
SWAPPED: Final = selection._resolve_prompt_swapped_provider  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L764-L790
INFO_AFTER_SWAP: Final = selection._deployment_model_info_after_prompt_swap  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L497-L502
ASYNC_OUTCOME: Final = selection._AsyncPromptManagementOutcome  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L505-L508
APPLY_PROMPT: Final[PromptApply] = cast(PromptApply, selection._apply_prompt_management_to_responses_call)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L793-L880
APPLY_FILES: Final[FilesApply] = cast(FilesApply, selection._apply_managed_file_id_mapping)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L934-L964


def selection_answer(case: SelectionCase) -> JsonValue:
    api_base = API_BASE(case["kwargs"])
    config = RESOLVE_CONFIG(case["model"], case["provider"], case["model_info"], api_base)
    kind: str | None = None
    if config is not None:
        provider: str = config.custom_llm_provider
        kind = "openai_like" if provider == "openai_like" else "native"
    return JSON.validate_python({
        "config": kind,
        "bridges": BRIDGES(config, case["use_chat_completions_api"]),
        "encrypted": ENCRYPTED_FAILURE(config, case["use_chat_completions_api"]),
        "passes": PASSES_THROUGH(case["model_info"]),
        "api_base": api_base,
        "bridge_kwargs": dict(BRIDGE_KWARGS(case["kwargs"], config, case["allowed_openai_params"])),
        "will_bridge": WILL_BRIDGE(
            case["model"],
            case["provider"],
            case["use_chat_completions_api"],
            case["model_info"],
            api_base,
        ),
    })


def prefixes_answer(case: PrefixesCase) -> JsonValue:
    kwargs: dict[str, object] = {}
    if case["flag_present"]:
        kwargs["use_chat_completions_api"] = case.get("flag")
    popped = POP_FLAG(kwargs)
    normalized, prefixed = NORMALIZE(case["model"])
    return JSON.validate_python({
        "normalized": [normalized, prefixed],
        "stripped": STRIP_PREFIX(case["model"]),
        "popped": popped,
        "remaining": kwargs,
    })


def compatible_answer(case: CompatibleCase) -> JsonValue:
    os.environ.clear()
    os.environ.update(case["environment"])
    params = GenericLiteLLMParams(api_key=case.get("api_key"))
    headers: dict[str, str] = CONFIG.validate_environment(dict(case["headers"]), "", params)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L30-L39
    return JSON.validate_python({
        "identity": CONFIG.custom_llm_provider,
        "native_file_search": CONFIG.supports_native_file_search(),
        "encrypted": CONFIG.supports_encrypted_agent_messages(),
        "headers": headers,
        "url": CONFIG.get_complete_url(case.get("api_base"), {}),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai_like/responses/transformation.py#L41-L51
    })


def credential_answer(case: CredentialCase) -> JsonValue:
    provider = SWAPPED(
        case["original_model"],
        case["swapped_model"],
        case.get("provider"),
        case["kwargs"],
        case.get("prompt_id"),
    )
    model_info = INFO_AFTER_SWAP(case.get("provider"), provider, case.get("model_info"))
    return JSON.validate_python({"provider": provider, "model_info": model_info})


def prompt_answer(case: PromptCallCase) -> JsonValue:
    kwargs: dict[str, object] = dict(case["kwargs"])
    outcome = case.get("async_outcome")
    if outcome is not None:
        kwargs[ASYNC_KEY] = ASYNC_OUTCOME(
            merged_optional_params=outcome["merged_optional_params"],
            deployment_model_info=outcome["deployment_model_info"],
        )
    manager = ScriptedLogging(case.get("hook")) if case["logging"] else None
    local_vars: dict[str, object] = {}
    input_value, model, provider, deployment = APPLY_PROMPT(
        case["input"],
        case["model"],
        case.get("provider"),
        manager,
        kwargs,
        local_vars,
        case["use_chat_completions_api"],
    )
    return JSON.validate_python({
        "input": input_value,
        "model": model,
        "provider": provider,
        "deployment_model_info": deployment,
        "locals": local_vars,
        "kwargs": kwargs,
        "requests": [] if manager is None else manager.requests,
        "calls": [] if manager is None else manager.calls,
    })


def file_answer(case: FileMappingCase) -> JsonValue:
    kwargs: dict[str, object] = {}
    model_info = case.get("model_info")
    if model_info is not None:
        kwargs["model_info"] = model_info
    mapping = case.get("mapping")
    if mapping is not None:
        kwargs["model_file_id_mapping"] = mapping
    local_vars: dict[str, object] = {}
    input_value, tools = APPLY_FILES(case["input"], case.get("tools"), kwargs, local_vars)
    return JSON.validate_python({"input": input_value, "tools": tools, "locals": local_vars})


def run(input_value: Input) -> JsonValue:
    match input_value["action"]:
        case "selection":
            return selection_answer(input_value)
        case "prefixes":
            return prefixes_answer(input_value)
        case "compatible":
            return compatible_answer(input_value)
        case "credential_boundary":
            return credential_answer(input_value)
        case "prompt_call":
            return prompt_answer(input_value)
        case "file_mapping":
            return file_answer(input_value)
