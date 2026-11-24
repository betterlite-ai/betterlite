import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, NamedTuple, TypedDict

import litellm
import litellm.router
import litellm.utils
from litellm.caching.dual_cache import DualCache
from litellm.router import Router
from litellm.router_utils.pre_call_checks import io_token_rate_limit_check
from litellm.types.router import ModelGroupInfo
from opentelemetry.trace import Span
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
    response: JsonValue
    request_kwargs: dict[str, JsonValue] | None
    metadata: PrincipalInput | None
    litellm_metadata: PrincipalInput | None
    team_id: str | None
    candidates: list[int] | None


class UsagePayload(TypedDict):
    tpm: int | None
    rpm: int | None
    reads: list[list[str]]


class IoPayload(TypedDict):
    itpm: int | None
    otpm: int | None
    reads: list[list[str]]


class RemainingPayload(TypedDict):
    headers: dict[str, int]
    reads: list[list[str]]


class ResponsePayload(TypedDict):
    response: JsonValue
    reads: list[list[str]]


class FilterPayload(TypedDict):
    ids: list[JsonValue]


class FilteredRow(TypedDict):
    model_info: dict[str, JsonValue]


class Grants(NamedTuple):
    models: list[str]
    team_models: list[str]


class FixedClock:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.epoch, tz=UTC)


class ScriptedCache(DualCache):
    def __init__(
        self,
        *,
        mode: Literal["missing", "values", "error"],
        values: list[JsonValue],
    ) -> None:
        super().__init__()
        self.mode = mode
        self.values = list(values)
        self.reads: list[list[str]] = []

    @override
    async def async_batch_get_cache(
        self,
        keys: list[str],
        parent_otel_span: Span | None = None,
        local_only: bool = False,
        throttle_redis: bool = True,
        **_kwargs: JsonValue,
    ) -> list[JsonValue] | None:
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
        super().__init__(model_list=model_list)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L772

    @override
    def get_model_group_info(self, model_group: str) -> ModelGroupInfo | None:
        _ = model_group
        return self.group_info


def group_info(input_value: GroupInfoInput | None) -> ModelGroupInfo | None:
    if input_value is None:
        return None
    return ModelGroupInfo(
        model_group="group",
        providers=list[str](),
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


def principal_bucket(principal: PrincipalInput | None) -> dict[str, Grants]:
    if principal is None:
        return {}
    return {
        "user_api_key_auth": Grants(principal["models"], principal["team_models"])
    }


def filter_request_kwargs(input_value: Input) -> dict[str, dict[str, Grants]]:
    return {
        "metadata": principal_bucket(input_value["metadata"]),
        "litellm_metadata": principal_bucket(input_value["litellm_metadata"]),
    }


def candidate_rows(input_value: Input, group: str) -> list[dict[str, JsonValue]]:
    rows = [
        row
        for row in input_value["model_list"]
        if row.get("model_name") == group
    ]
    indices = input_value["candidates"]
    if indices is None:
        return rows
    return [rows[index] for index in indices]


async def answer(input_value: Input, router: CaseRouter, cache: ScriptedCache) -> JsonValue:
    function = input_value["function"]
    if function == "usage":
        tpm, rpm = await router.get_model_group_usage(model_group(input_value, function))
        usage: UsagePayload = {"tpm": tpm, "rpm": rpm, "reads": cache.reads}
        return ANSWER.validate_python(usage)
    if function == "io_usage":
        itpm, otpm = await router.get_model_group_io_token_usage(
            model_group(input_value, function)
        )
        io: IoPayload = {"itpm": itpm, "otpm": otpm, "reads": cache.reads}
        return ANSWER.validate_python(io)
    if function == "remaining":
        headers = await router.get_remaining_model_group_usage(
            model_group(input_value, function)
        )
        remaining: RemainingPayload = {"headers": headers, "reads": cache.reads}
        return ANSWER.validate_python(remaining)
    if function == "response":
        returned: JsonValue = await router.set_response_headers(  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L11367
            response=input_value["response"],
            model_group=input_value["model_group"],
            request_kwargs=input_value["request_kwargs"],
        )
        response: ResponsePayload = {"response": returned, "reads": cache.reads}
        return ANSWER.validate_python(response)
    return filter_answer(input_value, router)


def filter_answer(input_value: Input, router: CaseRouter) -> JsonValue:
    group = model_group(input_value, "filter")
    filtered: list[FilteredRow] = router._filter_deployments_by_model_access_groups(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L12941
        model=group,
        healthy_deployments=candidate_rows(input_value, group),
        request_kwargs=filter_request_kwargs(input_value),
        request_team_id=input_value["team_id"],
    )
    ids: list[JsonValue] = [row["model_info"]["id"] for row in filtered]
    payload: FilterPayload = {"ids": ids}
    return ANSWER.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    cache = ScriptedCache(
        mode=input_value["cache"]["mode"], values=input_value["cache"]["values"]
    )
    clock = FixedClock(input_value["now"])
    original = litellm.utils.get_utc_datetime
    litellm.router.get_utc_datetime = clock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L335
    io_token_rate_limit_check.get_utc_datetime = clock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/io_token_rate_limit_check.py#L26
    try:
        router = CaseRouter(
            model_list=input_value["model_list"],
            info=group_info(input_value["group_info"]),
        )
        router.cache = cache
        return asyncio.run(answer(input_value, router, cache))
    finally:
        litellm.router.get_utc_datetime = original  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L335
        io_token_rate_limit_check.get_utc_datetime = original  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/io_token_rate_limit_check.py#L26
