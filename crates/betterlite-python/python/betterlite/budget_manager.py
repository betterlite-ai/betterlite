import json
import logging
import threading
import time
from collections.abc import Mapping, Sequence
from contextlib import suppress
from pathlib import Path
from typing import Final

import httpx2
from pydantic import JsonValue, TypeAdapter

import betterlite
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
_DURATION_ERROR: Final = (
    'duration needs to be one of ["daily", "weekly", "monthly", "yearly"]'
)
_INPUT_ERROR: Final = (
    "Either a chat completion object or the text response needs to be passed in. "
    "Learn more - https://docs.litellm.ai/docs/budget_manager"
)
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_USERS: Final[TypeAdapter[dict[str, dict[str, JsonValue]]]] = TypeAdapter(
    dict[str, dict[str, JsonValue]]
)

__all__ = ["BudgetManager", "module_level_client"]


class ModuleClient:
    _client: Final[httpx2.Client]

    def __init__(self) -> None:
        self._client = httpx2.Client()

    def post(self, url: str, *, headers: Mapping[str, str], json: JsonValue) -> JsonValue:
        response = self._client.post(url, headers=dict(headers), json=json)
        return _JSON.validate_json(response.text)


module_level_client: Final = ModuleClient()


class BudgetManager:
    client_type: str
    project_name: str
    api_base: str
    headers: dict[str, str]
    user_dict: dict[str, dict[str, JsonValue]]

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
        self.user_dict = {}
        self.load_data()

    def print_verbose(self, print_statement: str) -> None:
        with suppress(Exception):
            if betterlite.set_verbose:
                logging.info(print_statement)

    def load_data(self) -> None:
        match self.client_type:
            case "local":
                path = Path(_COST_FILE)
                if path.is_file():
                    self.user_dict = _USERS.validate_json(path.read_text())
                else:
                    self.print_verbose("User Dictionary not found!")
                    self.user_dict = {}
                self.print_verbose(f"user dict from local: {self.user_dict}")
            case "hosted":
                url = self.api_base + "/get_budget"
                request: dict[str, JsonValue] = {"project_name": self.project_name}
                reply = betterlite.module_level_client.post(
                    url, headers=self.headers, json=request
                )
                match reply:
                    case dict() as fields:
                        if fields["status"] == "error":
                            self.user_dict = {}
                        else:
                            self.user_dict = _USERS.validate_python(fields["data"])
                    case object():
                        raise TypeError("hosted budget reply must be an object")
            case object():
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
        self.user_dict[user] = {"total_budget": total_budget}
        if duration is None:
            return self.user_dict[user]
        duration_in_days = _DURATIONS.get(duration)
        if duration_in_days is None:
            raise ValueError(_DURATION_ERROR)
        self.user_dict[user] = {
            "total_budget": total_budget,
            "duration": duration_in_days,
            "created_at": created_at,
            "last_updated_at": created_at,
        }
        self._save_data_thread()
        return self.user_dict[user]

    def projected_cost(
        self, model: str, messages: Sequence[Mapping[str, JsonValue]], user: str
    ) -> float:
        parts: list[str] = []
        for message in messages:
            match message["content"]:
                case str() as content:
                    parts.append(content)
                case object():
                    raise TypeError("content must be a string")
        prompt_tokens = betterlite.token_counter(model=model, text="".join(parts))
        prompt_cost, _completion = betterlite.cost_per_token(
            model=model, prompt_tokens=prompt_tokens, completion_tokens=0
        )
        return prompt_cost + _cost_field(self.user_dict[user], "current_cost")

    def update_cost(
        self,
        user: str,
        completion_obj: JsonSource | None = None,
        model: str | None = None,
        input_text: str | None = None,
        output_text: str | None = None,
    ) -> dict[str, JsonValue]:
        if model is not None and input_text is not None and output_text is not None:
            prompt_tokens = betterlite.token_counter(
                model=model, messages=[{"role": "user", "content": input_text}]
            )
            completion_tokens = betterlite.token_counter(
                model=model, messages=[{"role": "user", "content": output_text}]
            )
            prompt_cost, completion_price = betterlite.cost_per_token(
                model=model,
                prompt_tokens=prompt_tokens,
                completion_tokens=completion_tokens,
            )
            cost = prompt_cost + completion_price
        elif completion_obj is not None:
            cost = betterlite.completion_cost(completion_response=completion_obj)
            match dump_object(completion_obj).get("model"):
                case str() as name:
                    model = name
                case object():
                    raise TypeError("model must be a string")
        else:
            raise ValueError(_INPUT_ERROR)
        record = self.user_dict[user]
        record["current_cost"] = cost + _cost_field(record, "current_cost")
        model_cost = record.get("model_cost")
        if model_cost is None:
            created: dict[str, JsonValue] = {model: cost}
            record["model_cost"] = created
        else:
            match model_cost:
                case dict() as model_costs:
                    model_costs[model] = cost + _cost_field(model_costs, model)
                case object():
                    raise TypeError("model_cost must be an object")
        self._save_data_thread()
        return {"user": record}

    def get_total_budget(self, user: str) -> JsonValue:
        record = self.user_dict[user]
        return record["total_budget"]

    def get_current_cost(self, user: str) -> JsonValue:
        value = self.user_dict[user].get("current_cost")
        return 0 if value is None else value

    def get_model_cost(self, user: str) -> JsonValue:
        value = self.user_dict[user].get("model_cost")
        return 0 if value is None else value

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
        return {"user": record}

    def reset_on_duration(self, user: str) -> None:
        record = self.user_dict[user]
        last_updated_at = _number(record["last_updated_at"], "last_updated_at")
        current_time = time.time()
        duration_in_seconds = (
            _number(record["duration"], "duration") * HOURS_IN_A_DAY * 60 * 60
        )
        if current_time - last_updated_at >= duration_in_seconds:
            self.reset_cost(user)
            record["last_updated_at"] = current_time
            self._save_data_thread()

    def update_budget_all_users(self) -> None:
        for user in self.get_users():
            if "duration" in self.user_dict[user]:
                self.reset_on_duration(user)

    def _save_data_thread(self) -> None:
        thread = threading.Thread(target=self.save_data)
        thread.start()

    def save_data(self) -> JsonValue:
        if self.client_type == "local":
            Path(_COST_FILE).write_text(json.dumps(self.user_dict, indent=4))
            return {"status": "success"}
        request: dict[str, JsonValue] = {
            "project_name": self.project_name,
            "user_dict": _JSON.validate_python(self.user_dict),
        }
        return betterlite.module_level_client.post(
            self.api_base + "/set_budget", headers=self.headers, json=request
        )


def _number(value: JsonValue, name: str) -> float:
    match value:
        case (bool() | int() | float()) as number:
            return float(number)
        case object():
            raise TypeError(f"{name} must be a number")


def _cost_field(record: dict[str, JsonValue], name: str) -> float:
    value = record.get(name)
    if value is None:
        return 0.0
    return _number(value, name)
