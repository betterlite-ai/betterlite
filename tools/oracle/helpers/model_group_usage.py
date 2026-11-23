import asyncio
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Final, Literal, TypedDict
from unittest import mock

import litellm
from litellm.caching.dual_cache import DualCache
from litellm.router import Router
from litellm.types.router import DeploymentTypedDict, ModelGroupInfo
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ANSWER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class GroupInfoInput(TypedDict):
    tpm: int | None
    rpm: int | None
    itpm: int | None
    otpm: int | None


class CacheInput(TypedDict):
    mode: Literal["missing", "values", "error"]
    values: list[JsonValue]


class PrincipalInput(TypedDict):
    models: list[str]
    team_models: list[str]


class Input(TypedDict):
    function: Literal["usage", "io_usage", "remaining", "response", "filter"]
    model_group: str | None
    now: int
    group_info: GroupInfoInput | None
    cache: CacheInput
    model_list: list[dict[str, JsonValue]]
    response: dict[str, object] | None
    request_kwargs: dict[str, object] | None
    metadata: PrincipalInput | None
    litellm_metadata: PrincipalInput | None
    team_id: str | None
    candidates: list[int] | None


class FilteredDeployment(TypedDict):
    model_info: dict[str, JsonValue]


class FixedClock:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.epoch, tz=UTC)


class ScriptedCache(DualCache):
    def __init__(self, *, mode: str, values: list[JsonValue]) -> None:
        super().__init__()
        self.mode = mode
        self.values = list[object | None](values)
        self.reads: list[list[str]] = []

    @override
    async def async_batch_get_cache(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L401
        self,
        keys: list[str],
        parent_otel_span: object = None,
        local_only: bool = False,
        throttle_redis: bool = True,
        **_kwargs: object,
    ) -> list[object | None] | None:
        _ = parent_otel_span, local_only, throttle_redis
        self.reads.append(keys)
        if self.mode == "error":
            raise RuntimeError("the scripted counter read failed")
        if self.mode == "missing":
            return None
        return self.values


class CaseRouter(Router):
    def __init__(
        self, model_list: list[dict[str, JsonValue]], info: ModelGroupInfo | None
    ) -> None:
        self.group_info = info
        super().__init__(model_list=model_list)

    @override
    def get_model_group_info(self, model_group: str) -> ModelGroupInfo | None:
        _ = model_group
        return self.group_info


def group_info(input_value: GroupInfoInput | None) -> ModelGroupInfo | None:
    if input_value is None:
        return None
    return ModelGroupInfo(
        model_group="",
        providers=[],
        tpm=input_value["tpm"],
        rpm=input_value["rpm"],
        itpm=input_value["itpm"],
        otpm=input_value["otpm"],
    )


def model_group(input_value: Input, function: str) -> str:
    group = input_value["model_group"]
    if group is None:
        raise ValueError(f"the {function} function needs a model_group")
    return group


def principal_bucket(principal: PrincipalInput | None) -> dict[str, object]:
    if principal is None:
        return {}
    return {
        "user_api_key_auth": SimpleNamespace(
            models=principal["models"], team_models=principal["team_models"]
        )
    }


def filter_request_kwargs(input_value: Input) -> dict[str, object]:
    return {
        "metadata": principal_bucket(input_value["metadata"]),
        "litellm_metadata": principal_bucket(input_value["litellm_metadata"]),
    }


def candidate_deployments(
    input_value: Input, router: CaseRouter, group: str
) -> list[DeploymentTypedDict]:
    rows = router.get_model_list(model_name=group) or []
    indices = input_value["candidates"]
    if indices is None:
        return rows
    return [rows[index] for index in indices]


async def usage_answer(
    input_value: Input, router: CaseRouter, cache: ScriptedCache
) -> JsonValue:
    tpm, rpm = await router.get_model_group_usage(model_group(input_value, "usage"))
    return ANSWER.validate_python({"tpm": tpm, "rpm": rpm, "reads": cache.reads})


async def io_answer(
    input_value: Input, router: CaseRouter, cache: ScriptedCache
) -> JsonValue:
    itpm, otpm = await router.get_model_group_io_token_usage(
        model_group(input_value, "io_usage")
    )
    return ANSWER.validate_python({"itpm": itpm, "otpm": otpm, "reads": cache.reads})


async def remaining_answer(
    input_value: Input, router: CaseRouter, cache: ScriptedCache
) -> JsonValue:
    headers = await router.get_remaining_model_group_usage(
        model_group(input_value, "remaining")
    )
    return ANSWER.validate_python({"headers": headers, "reads": cache.reads})


async def response_answer(
    input_value: Input, router: CaseRouter, cache: ScriptedCache
) -> JsonValue:
    returned: object = await router.set_response_headers(
        response=input_value["response"],
        model_group=input_value["model_group"],
        request_kwargs=input_value["request_kwargs"],
    )
    return ANSWER.validate_python({"response": returned, "reads": cache.reads})


def filter_answer(input_value: Input, router: CaseRouter) -> JsonValue:
    group = model_group(input_value, "filter")
    deployments = candidate_deployments(input_value, router, group)
    filtered: list[FilteredDeployment] = router._filter_deployments_by_model_access_groups(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L12941
        model=group,
        healthy_deployments=deployments,
        request_kwargs=filter_request_kwargs(input_value),
        request_team_id=input_value["team_id"],
    )
    ids: list[JsonValue] = [entry["model_info"]["id"] for entry in filtered]
    return ANSWER.validate_python({"ids": ids})


async def answer(input_value: Input, router: CaseRouter, cache: ScriptedCache) -> JsonValue:
    function = input_value["function"]
    if function == "usage":
        return await usage_answer(input_value, router, cache)
    if function == "io_usage":
        return await io_answer(input_value, router, cache)
    if function == "remaining":
        return await remaining_answer(input_value, router, cache)
    if function == "response":
        return await response_answer(input_value, router, cache)
    return filter_answer(input_value, router)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    cache = ScriptedCache(
        mode=input_value["cache"]["mode"], values=input_value["cache"]["values"]
    )
    clock = FixedClock(input_value["now"])
    with (
        mock.patch("litellm.router.get_utc_datetime", clock),
        mock.patch(
            "litellm.router_utils.pre_call_checks.io_token_rate_limit_check.get_utc_datetime",
            clock,
        ),
    ):
        router = CaseRouter(
            model_list=input_value["model_list"],
            info=group_info(input_value["group_info"]),
        )
        router.cache = cache
        return asyncio.run(answer(input_value, router, cache))
