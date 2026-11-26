import asyncio
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypedDict

from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import CallInfo, UserAPIKeyAuth
from litellm.proxy.utils import ProxyLogging
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

Function = Literal[
    "key_fallback",
    "user_budget",
    "team_budget",
    "end_user_apply",
    "end_user_update",
    "global_spend",
]


class Call(TypedDict):
    function: Function
    token: NotRequired[dict[str, JsonValue] | None]
    models: NotRequired[list[str] | None]
    model_name: NotRequired[str | None]
    request_model: NotRequired[str | None]
    request_data: NotRequired[dict[str, JsonValue] | None]
    path_params: NotRequired[dict[str, JsonValue] | None]
    failing: NotRequired[bool | None]
    fallback: NotRequired[str | None]
    failing_models: NotRequired[list[str] | None]
    budget: NotRequired[dict[str, JsonValue] | None]
    params: NotRequired[dict[str, JsonValue] | None]
    end_user_id: NotRequired[str | None]
    max_budget: NotRequired[float | None]
    spend: NotRequired[float | None]
    database: NotRequired[bool | None]
    admin_name: NotRequired[str | None]
    tokens: NotRequired[list[str] | None]
    aliases: NotRequired[dict[str, str] | None]
    max_recurse_depth: NotRequired[int | None]


class Request(TypedDict):
    calls: list[Call]


def budget_error(entity_type: str, entity_id: str) -> BudgetExceededError:
    return BudgetExceededError(
        current_cost=12.5,
        max_budget=10.0,
        message="Budget has been exceeded!",
        llm_provider="openai",
        entity_type=entity_type,
        entity_id=entity_id,
    )


def budget_verdict(*, failing: bool, entity_type: str, entity_id: str) -> bool:
    if failing:
        raise budget_error(entity_type, entity_id)
    return True


class ScriptedKeyLimiter:
    def __init__(self, *, failing: bool | None, fallback: str | None) -> None:
        self.failing: Final = failing
        self.fallback: Final = fallback
        self.calls: Final[list[str]] = []

    async def is_key_within_model_budget(
        self, user_api_key_dict: UserAPIKeyAuth, model: str
    ) -> bool:
        del user_api_key_dict
        self.calls.append(f"key:{model}")
        return budget_verdict(failing=bool(self.failing), entity_type="key", entity_id="hashed-key")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L450-L453

    async def get_fallback_model_within_budget(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        model: str,
    ) -> str | None:
        del user_api_key_dict
        self.calls.append(f"fallback:{model}")
        return self.fallback  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L455-L458


class ScriptedUserLimiter:
    def __init__(self, failing_models: list[str]) -> None:
        self.failing_models: Final = failing_models
        self.calls: Final[list[JsonValue]] = []

    async def is_user_within_model_budget(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L391-L395
        self,
        user_id: str,
        user_model_max_budget: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        model: str,
    ) -> bool:
        record: JsonValue = {
            "user_id": user_id,
            "model": model,
            "budget": OBJECT.validate_python(user_model_max_budget),
        }
        self.calls.append(record)
        return budget_verdict(
            failing=model in self.failing_models, entity_type="user", entity_id=user_id
        )


class ScriptedTeamLimiter:
    def __init__(self, failing_models: list[str]) -> None:
        self.failing_models: Final = failing_models
        self.calls: Final[list[JsonValue]] = []

    async def is_team_within_model_budget(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L409-L414
        self,
        team_id: str,
        team_model_max_budget: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        key_model_max_budget: Mapping[str, object] | None,  # ast-grep-ignore: no-loose-types
        model: str,
    ) -> bool:
        key_budget: JsonValue = None
        if key_model_max_budget is not None:
            key_budget = OBJECT.validate_python(key_model_max_budget)
        record: JsonValue = {
            "team_id": team_id,
            "model": model,
            "team_budget": OBJECT.validate_python(team_model_max_budget),
            "key_budget": key_budget,
        }
        self.calls.append(record)
        return budget_verdict(
            failing=model in self.failing_models, entity_type="team", entity_id=team_id
        )


class SpendRow:
    def __init__(self, spend: float) -> None:
        self.spend: Final = spend


class FakeUserTable:
    def __init__(self, *, spend: float | None, gated: bool) -> None:
        self.spend: Final = spend
        self.gated: Final = gated
        self.reads: Final[list[str]] = []
        self.started: Final = asyncio.Event()
        self.release: Final = asyncio.Event()

    async def find_unique(self, *, where: dict[str, str]) -> SpendRow | None:
        del where
        self.reads.append("proxy_budget")
        self.started.set()
        if self.gated:
            await self.release.wait()
        if self.spend is None:
            return None
        return SpendRow(self.spend)


class FakeDatabase:
    def __init__(self, usertable: FakeUserTable) -> None:
        self.litellm_usertable: Final = usertable


class FakePrisma:
    def __init__(self, usertable: FakeUserTable) -> None:
        self.db: Final = FakeDatabase(usertable)


class FakeCache:
    def __init__(self) -> None:
        self.stored: float | None = None

    async def async_get_cache(self, *, key: str) -> float | None:
        del key
        return self.stored

    async def async_set_cache(self, *, key: str, value: float | None) -> None:
        del key
        self.stored = value


class AlertRecorder(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: Final[list[JsonValue]] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        dumped = OBJECT.validate_json(user_info.model_dump_json())
        self.alerts.append(
            {
                "type": type,
                "info": {
                    "user_id": dumped.get("user_id"),
                    "max_budget": dumped.get("max_budget"),
                    "spend": dumped.get("spend"),
                    "token": dumped.get("token"),
                    "event_group": dumped.get("event_group"),
                },
            }
        )
