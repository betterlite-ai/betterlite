import asyncio
from collections.abc import Mapping
from typing import Final

from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import CallInfo, UserAPIKeyAuth
from litellm.proxy.utils import ProxyLogging
from pydantic import JsonValue
from typing_extensions import override

from .wire import OBJECT


def budget_error(entity_type: str, entity_id: str) -> BudgetExceededError:
    return BudgetExceededError(
        current_cost=12.5,
        max_budget=10.0,
        message="Budget has been exceeded!",
        llm_provider="openai",
        entity_type=entity_type,
        entity_id=entity_id,
    )


class ScriptedKeyLimiter:
    def __init__(self, *, failing: bool | None, fallback: str | None) -> None:
        self.failing: Final = failing
        self.fallback: Final = fallback
        self.calls: Final[list[str]] = []

    async def is_key_within_model_budget(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L450-L453
        self,
        user_api_key_dict: UserAPIKeyAuth,
        model: str,
    ) -> bool:
        del user_api_key_dict
        self.calls.append(f"key:{model}")
        if self.failing:
            raise budget_error("key", "hashed-key")
        return True

    async def get_fallback_model_within_budget(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        model: str,
    ) -> str | None:
        del user_api_key_dict
        self.calls.append(f"fallback:{model}")
        return self.fallback


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
        self.calls.append(
            {
                "user_id": user_id,
                "model": model,
                "budget": OBJECT.validate_python(user_model_max_budget),
            }
        )
        if model in self.failing_models:
            raise budget_error("user", user_id)
        return True


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
        self.calls.append(
            {
                "team_id": team_id,
                "model": model,
                "team_budget": OBJECT.validate_python(team_model_max_budget),
                "key_budget": key_budget,
            }
        )
        if model in self.failing_models:
            raise budget_error("team", team_id)
        return True


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
