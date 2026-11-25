import json
import logging
import threading
import time
from collections.abc import Mapping, Sequence
from contextlib import suppress
from decimal import Decimal
from pathlib import Path
from typing import Final, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite._core_helpers.budget_http import ModuleClient
from betterlite._core_helpers.budget_records import (
    USERS,
    BudgetRecord,
    cost_decimal,
    public_money,
    record_json,
)
from betterlite._core_helpers.values import text_of
from betterlite._cost.objects import dump_object
from betterlite._cost.wire import JsonSource
from betterlite._settings.env import env_number

HOURS_IN_A_DAY: Final[int] = env_number("HOURS_IN_A_DAY", 24, int)
DAYS_IN_A_WEEK: Final[int] = env_number("DAYS_IN_A_WEEK", 7, int)
DAYS_IN_A_MONTH: Final[int] = env_number("DAYS_IN_A_MONTH", 28, int)
DAYS_IN_A_YEAR: Final[int] = env_number("DAYS_IN_A_YEAR", 365, int)
_DEFAULT_API_BASE: Final = "https://api.litellm.ai"
_DEFAULT_HEADERS: Final[dict[str, str]] = {"Content-Type": "application/json"}
_DURATIONS: Final[dict[str, int]] = {
    "daily": 1,
    "weekly": DAYS_IN_A_WEEK,
    "monthly": DAYS_IN_A_MONTH,
    "yearly": DAYS_IN_A_YEAR,
}
_COST_FILE: Final = "user_cost.json"
_CREATED_AT_DEFAULT: Final = time.time()
_DURATION_ERROR: Final = 'duration needs to be one of ["daily", "weekly", "monthly", "yearly"]'
_INPUT_ERROR: Final = (
    "Either a chat completion object or the text response needs to be passed in. "
    "Learn more - https://docs.litellm.ai/docs/budget_manager"
)
_LOGGER: Final = logging.getLogger(__name__)
_COST_FILE_LOCK: Final = threading.Lock()


class HostedReply(TypedDict):
    status: str
    data: NotRequired[dict[str, BudgetRecord]]


_REPLY: Final[TypeAdapter[HostedReply]] = TypeAdapter(HostedReply)

__all__ = ["BudgetManager", "ModuleClient", "module_level_client"]


module_level_client: Final = ModuleClient()


class BudgetManager:
    user_dict: dict[str, BudgetRecord]

    def __init__(
        self,
        project_name: str,
        client_type: str = "local",
        api_base: str | None = None,
        headers: dict[str, str] | None = None,
    ) -> None:
        self.client_type = client_type
        self.project_name = project_name
        self.api_base = _DEFAULT_API_BASE if api_base is None else api_base
        self.headers = dict(_DEFAULT_HEADERS) if headers is None else dict(headers)
        self._save_lock = threading.Lock()
        self.user_dict = {}
        self.load_data()

    def print_verbose(self, print_statement: str) -> None:
        with suppress(Exception):
            if betterlite.set_verbose:
                _LOGGER.info(print_statement)

    def load_data(self) -> None:
        match self.client_type:
            case "local":
                path = Path(_COST_FILE)
                if path.is_file():
                    self.user_dict = USERS.validate_json(path.read_text())
                else:
                    self.print_verbose("User Dictionary not found!")
                    self.user_dict = {}
                self.print_verbose(f"user dict from local: {self.user_dict}")
            case "hosted":
                url = self.api_base + "/get_budget"
                request: dict[str, JsonValue] = {"project_name": self.project_name}
                parsed = betterlite.module_level_client.post(
                    url, headers=self.headers, json=request
                )
                reply = _REPLY.validate_python(parsed)
                if reply["status"] == "error":
                    self.user_dict = {}
                else:
                    if "data" not in reply:
                        raise KeyError("data")
                    self.user_dict = reply["data"]
            case str():
                raise ValueError(
                    f"client_type must be 'local' or 'hosted', got {self.client_type!r}"
                )

    def create_budget(
        self,
        total_budget: float,
        user: str,
        duration: str | None = None,
        created_at: float = _CREATED_AT_DEFAULT,
    ) -> dict[str, JsonValue]:
        record: BudgetRecord = {"total_budget": Decimal(str(total_budget))}
        self.user_dict[user] = record
        if duration is None:
            return record_json(record)
        duration_in_days = _DURATIONS.get(duration)
        if duration_in_days is None:
            raise ValueError(_DURATION_ERROR)
        record["duration"] = duration_in_days
        record["created_at"] = created_at
        record["last_updated_at"] = created_at
        self._save_data_thread()
        return record_json(record)

    def projected_cost(
        self, model: str, messages: Sequence[Mapping[str, JsonValue]], user: str
    ) -> float:
        parts: list[str] = []
        for message in messages:
            match message["content"]:
                case str() as content:
                    parts.append(content)
                case bool() | int() | float() | list() | dict() | None:
                    raise TypeError("content must be a string")
        prompt_tokens = betterlite.token_counter(model=model, text="".join(parts))
        prompt_cost, _completion = betterlite.cost_per_token(
            model=model, prompt_tokens=prompt_tokens, completion_tokens=0
        )
        stored = self.user_dict[user].get("current_cost")
        return float(Decimal(str(prompt_cost)) + cost_decimal(stored))

    def update_cost(
        self,
        user: str,
        completion_obj: JsonSource | None = None,
        model: str | None = None,
        input_text: str | None = None,
        output_text: str | None = None,
    ) -> dict[str, JsonValue]:
        if model is not None and input_text is not None and output_text is not None:
            amount = _text_cost(model, input_text, output_text)
        elif completion_obj is not None:
            amount = Decimal(str(betterlite.completion_cost(completion_response=completion_obj)))
            model = _written_model(completion_obj)
        else:
            raise ValueError(_INPUT_ERROR)
        record = self.user_dict[user]
        record["current_cost"] = amount + cost_decimal(record.get("current_cost"))
        model_cost = record.get("model_cost")
        if model_cost is None:
            record["model_cost"] = {model: amount}
        else:
            model_cost[model] = amount + cost_decimal(model_cost.get(model))
        self._save_data_thread()
        return {"user": record_json(record)}

    def get_total_budget(self, user: str) -> JsonValue:
        record = self.user_dict[user]
        if "total_budget" not in record:
            raise KeyError("total_budget")
        return float(record["total_budget"])

    def get_current_cost(self, user: str) -> JsonValue:
        value = self.user_dict[user].get("current_cost")
        if value is None:
            return 0
        return public_money(value)

    def get_model_cost(self, user: str) -> JsonValue:
        value = self.user_dict[user].get("model_cost")
        if value is None:
            return 0
        return {name: float(cost) for name, cost in value.items()}

    # ast-grep-ignore: no-one-line-functions
    def is_valid_user(self, user: str) -> bool:
        return user in self.user_dict

    # ast-grep-ignore: no-one-line-functions
    def get_users(self) -> list[str]:
        return list(self.user_dict)

    def reset_cost(self, user: str) -> dict[str, JsonValue]:
        record = self.user_dict[user]
        record["current_cost"] = 0
        record["model_cost"] = {}
        return {"user": record_json(record)}

    def reset_on_duration(self, user: str) -> None:
        record = self.user_dict[user]
        now = time.time()
        if "last_updated_at" not in record:
            raise KeyError("last_updated_at")
        if "duration" not in record:
            raise KeyError("duration")
        last_updated_at = record["last_updated_at"]
        duration = record["duration"]
        elapsed = now - last_updated_at
        if elapsed < duration * HOURS_IN_A_DAY * 60 * 60:
            return
        self.reset_cost(user)
        record["last_updated_at"] = now
        self._save_data_thread()

    def update_budget_all_users(self) -> None:
        for user in self.get_users():
            if "duration" in self.user_dict[user]:
                self.reset_on_duration(user)

    def _save_data_thread(self) -> None:
        thread = threading.Thread(target=self.save_data)
        thread.start()

    def save_data(self) -> JsonValue:
        with self._save_lock:
            users: dict[str, JsonValue] = {
                user: record_json(record) for user, record in self.user_dict.items()
            }
            if self.client_type == "local":
                pending = Path(f"{_COST_FILE}.pending")
                with _COST_FILE_LOCK:
                    pending.write_text(json.dumps(users, indent=4))
                    pending.replace(_COST_FILE)
                return {"status": "success"}
            request: dict[str, JsonValue] = {
                "project_name": self.project_name,
                "user_dict": users,
            }
            return betterlite.module_level_client.post(
                self.api_base + "/set_budget", headers=self.headers, json=request
            )


def _written_model(completion_obj: JsonSource) -> str:
    name = text_of(dump_object(completion_obj).get("model"))
    if name is None:
        raise TypeError("model must be a string")
    return name


def _text_cost(model: str, input_text: str, output_text: str) -> Decimal:
    prompt_tokens = betterlite.token_counter(
        model=model, messages=[{"role": "user", "content": input_text}]
    )
    completion_tokens = betterlite.token_counter(
        model=model, messages=[{"role": "user", "content": output_text}]
    )
    prompt_cost, completion_price = betterlite.cost_per_token(
        model=model, prompt_tokens=prompt_tokens, completion_tokens=completion_tokens
    )
    return Decimal(str(prompt_cost)) + Decimal(str(completion_price))
