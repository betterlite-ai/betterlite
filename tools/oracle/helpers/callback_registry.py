from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, cast

import litellm
import litellm.litellm_core_utils.logging_callback_manager as callback_manager
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.logging_callback_manager import LoggingCallbackManager
from litellm.proxy.common_utils import callback_config_validation as validation

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
AttributeValue: TypeAlias = bool | int | str
ListName: TypeAlias = Literal[
    "callbacks", "success_callback", "failure_callback", "input_callback", "service_callback"
]
DEFAULT_MAX_CALLBACKS: Final = 100


class InputError(ValueError):
    pass


class RemoveFromList(Protocol):
    def __call__(
        self, callback_list: list[CustomLogger], obj: CustomLogger, *, require_self: bool
    ) -> None: ...


class RemoveByType(Protocol):
    def __call__(
        self, callback_list: list[CustomLogger], callback_type: type[CustomLogger]
    ) -> None: ...


def target(name: ListName) -> list[CustomLogger]:
    match name:
        case "callbacks":
            return cast("list[CustomLogger]", litellm.callbacks)
        case "success_callback":
            return cast("list[CustomLogger]", litellm.success_callback)
        case "failure_callback":
            return cast("list[CustomLogger]", litellm.failure_callback)
        case "input_callback":
            return cast("list[CustomLogger]", litellm.input_callback)
        case "service_callback":
            return cast("list[CustomLogger]", litellm.service_callback)


ADD_METHODS: dict[ListName, str] = {
    "callbacks": "add_litellm_callback",
    "success_callback": "add_litellm_success_callback",
    "failure_callback": "add_litellm_failure_callback",
    "input_callback": "add_litellm_input_callback",
    "service_callback": "add_litellm_service_callback",
}


def add(manager: LoggingCallbackManager, name: ListName, callback: CustomLogger) -> None:
    method = cast(Callable[[CustomLogger], None], getattr(manager, ADD_METHODS[name]))
    method(callback)


def reset() -> None:
    litellm.callbacks = []
    litellm.success_callback = []
    litellm.failure_callback = []
    litellm.input_callback = []
    litellm.service_callback = []


def as_json(values: list[str]) -> list[Json]:
    converted: list[Json] = []
    converted.extend(values)
    return converted


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def string_map(value: Json, name: str) -> dict[str, str]:
    texts: dict[str, str] = {}
    for key, item in object_value(value, name).items():
        if not isinstance(item, str):
            raise InputError(f"{name}.{key} must be a string")
        texts[key] = item
    return texts


def attribute_value(value: Json, name: str) -> AttributeValue:
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return value
    raise InputError(f"{name} must be a string, integer or boolean")


def list_name(value: Json) -> ListName:
    if value == "callbacks":
        return "callbacks"
    if value == "success_callback":
        return "success_callback"
    if value == "failure_callback":
        return "failure_callback"
    if value == "input_callback":
        return "input_callback"
    if value == "service_callback":
        return "service_callback"
    raise InputError(f"unknown callback list: {value}")


class CallbackSpec:
    def __init__(self, value: Json) -> None:
        spec = object_value(value, "callback")
        class_name = spec.get("class_name")
        if not isinstance(class_name, str):
            raise InputError("callback.class_name must be a string")
        self.class_name = class_name
        self.attributes: dict[str, AttributeValue] = {}
        attributes = spec.get("attributes", {})
        for name, item in object_value(attributes, "callback.attributes").items():
            self.attributes[name] = attribute_value(item, f"callback.attributes.{name}")


class Session:
    def __init__(self, maximum: int) -> None:
        self.manager = LoggingCallbackManager()
        self.maximum = maximum
        key_member = "_get_custom_logger_key"
        remove_member = "remove_callback_from_list_by_object"
        remove_type_member = "remove_callbacks_by_type"
        self.key_of = cast(Callable[[CustomLogger], str], getattr(self.manager, key_member))
        self.remove_from_list = cast(
            RemoveFromList, getattr(self.manager, remove_member)
        )
        self.remove_by_type = cast(RemoveByType, getattr(self.manager, remove_type_member))
        self.classes: dict[str, type[CustomLogger]] = {}
        self.instances: dict[tuple[ListName, str], CustomLogger] = {}

    def build(self, spec: CallbackSpec) -> CustomLogger:
        cls = self.classes.get(spec.class_name)
        if cls is None:
            cls = cast(type[CustomLogger], type(spec.class_name, (CustomLogger,), {}))
            self.classes[spec.class_name] = cls
        instance = object.__new__(cls)
        for name, value in spec.attributes.items():
            setattr(instance, name, value)
        return instance

    def register(self, name: ListName, spec: CallbackSpec) -> Json:
        callback = self.build(spec)
        key = self.key_of(callback)
        table = target(name)
        before = len(table)
        present = any(self.key_of(existing) == key for existing in table)
        if before >= self.maximum:
            outcome = "skipped_capacity"
        elif present:
            outcome = "already_registered"
        else:
            outcome = "registered"
        add(self.manager, name, callback)
        expected = before + 1 if outcome == "registered" else before
        if len(table) != expected:
            raise InputError(f"{name} changed by {len(table) - before}, expected {outcome}")
        if outcome == "registered":
            self.instances[(name, key)] = callback
        return {"outcome": outcome}

    def remove(self, name: ListName, spec: CallbackSpec) -> Json:
        key = self.key_of(self.build(spec))
        table = target(name)
        existing = self.instances.pop((name, key), None)
        if existing is None:
            return {"outcome": "absent"}
        before = len(table)
        self.remove_from_list(table, existing, require_self=False)
        removed = len(table) < before
        return {"outcome": "removed" if removed else "absent"}

    def remove_class(self, name: ListName, class_name: str) -> Json:
        cls = self.classes.get(class_name)
        if cls is None:
            return {"outcome": "removed", "count": 0}
        table = target(name)
        before = len(table)
        self.remove_by_type(table, cls)
        stale = [
            slot
            for slot, instance in self.instances.items()
            if slot[0] == name and instance not in table
        ]
        for slot in stale:
            del self.instances[slot]
        return {"outcome": "removed", "count": before - len(table)}

    def validate(self, step: dict[str, Json]) -> Json:
        callback_name = step.get("callback_name")
        if callback_name is not None and not isinstance(callback_name, str):
            raise InputError("callback_name must be a string or null")
        variables = string_map(step.get("vars", {}), "vars")
        stored_value = step.get("stored", [])
        if not isinstance(stored_value, list):
            raise InputError("stored must be a list")
        stored = [string_map(entry, "stored") for entry in stored_value]
        message = validation.callback_config_error(callback_name, variables)
        if message is not None:
            return {"outcome": "invalid", "validator": "callback_config_error"}
        message = validation.cross_entry_family_error(variables, stored)
        if message is not None:
            return {"outcome": "invalid", "validator": "cross_entry_family_error"}
        message = validation.conflicting_span_scope_error(variables, stored)
        if message is not None:
            return {"outcome": "invalid", "validator": "conflicting_span_scope_error"}
        return {"outcome": "ok"}

    def run_step(self, value: Json) -> Json:
        step = object_value(value, "step")
        operation = step.get("op")
        if operation == "register":
            return self.register(list_name(step.get("list")), CallbackSpec(step.get("callback")))
        if operation == "remove_from_list":
            return self.remove(list_name(step.get("list")), CallbackSpec(step.get("callback")))
        if operation == "remove_class_from_list":
            class_name = step.get("class_name")
            if not isinstance(class_name, str):
                raise InputError("class_name must be a string")
            return self.remove_class(list_name(step.get("list")), class_name)
        if operation == "validate":
            return self.validate(step)
        raise InputError(f"unknown callback_registry operation: {operation}")

    def keys(self, name: ListName) -> list[str]:
        return [self.key_of(callback) for callback in target(name)]

    def lists(self) -> Json:
        return {
            "callbacks": as_json(self.keys("callbacks")),
            "success_callback": as_json(self.keys("success_callback")),
            "failure_callback": as_json(self.keys("failure_callback")),
            "input_callback": as_json(self.keys("input_callback")),
            "service_callback": as_json(self.keys("service_callback")),
        }

    def phases(self) -> Json:
        success = set(self.keys("success_callback"))
        general = set(self.keys("callbacks"))
        return {
            "input": as_json(sorted(self.keys("input_callback"))),
            "success": as_json(sorted(success | general)),
            "failure": as_json(sorted(set(self.keys("failure_callback")) | general)),
            "stream": as_json(sorted(success | general)),
            "service": as_json(sorted(self.keys("service_callback"))),
        }

    def selection(self) -> Json:
        grouped = self.manager.get_callbacks_by_type()
        return {
            "success": as_json(sorted(set(grouped["success"]))),
            "failure": as_json(sorted(set(grouped["failure"]))),
            "success_and_failure": as_json(sorted(set(grouped["success_and_failure"]))),
        }

    def observe(self, steps: list[Json]) -> Json:
        return {
            "steps": [self.run_step(step) for step in steps],
            "lists": self.lists(),
            "phases": self.phases(),
            "selection": self.selection(),
        }


def run(input_value: Json) -> Json:
    request = object_value(input_value, "input")
    maximum = request.get("max_callbacks", DEFAULT_MAX_CALLBACKS)
    if not isinstance(maximum, int) or isinstance(maximum, bool):
        raise InputError("max_callbacks must be an integer")
    if maximum != callback_manager.MAX_CALLBACKS:
        raise InputError(
            f"LITELLM_MAX_CALLBACKS resolves to {callback_manager.MAX_CALLBACKS}, not {maximum}"
        )
    steps = request.get("steps")
    if not isinstance(steps, list):
        raise InputError("steps must be a list")
    reset()
    return Session(maximum).observe(steps)
