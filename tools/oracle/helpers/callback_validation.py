import os
import re
import sys
from collections.abc import Callable, Generator, Mapping, Sequence
from contextlib import contextmanager
from importlib.abc import Loader, MetaPathFinder
from importlib.machinery import ModuleSpec
from importlib.util import find_spec
from types import ModuleType
from typing import NamedTuple, TypeAlias, cast, override

from litellm.integrations.otel.model.config import is_otel_v2_enabled
from litellm.proxy.common_utils import callback_config_validation as validation

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
STEP_FIELDS = {"function", "callback_name", "callback_vars", "stored", "metadata", "otel_v2"}
VarsCheck = Callable[[Mapping[str, str]], str | None]
NamedCheck = Callable[[str | None, Mapping[str, str]], str | None]
ERROR_KINDS = (
    ("sampling_rate_callback", r"^(?P<var>\w+) applies to the arize callback only"),
    ("sampling_rate_value", r"^(?P<var>\w+) must be a number between"),
    ("langfuse_environment", r"^Invalid langfuse_environment"),
    ("span_scope_callback", r"^langfuse_span_scope applies to"),
    ("span_scope_value", r"^Invalid langfuse_span_scope"),
    ("newrelic_requires_otel_v2", r"^Per-team New Relic routing requires"),
    ("newrelic_region", r"^Unknown newrelic_region"),
    ("newrelic_region_needs_key", r"^newrelic_region requires"),
    (
        "family_conflict",
        (
            r"^(?P<family>.+) is already configured by another callback entry on this team\. "
            r"Remove that entry before setting (?P<var>\w+) here"
        ),
    ),
    ("span_scope_conflict", r"^langfuse_span_scope is already set to"),
)

SDK_PREFIXES = ("opentelemetry.sdk", "opentelemetry.exporter")


class InputError(ValueError):
    pass


class StubModule(ModuleType):
    def __getattr__(self, name: str) -> type:
        if name.startswith("__"):
            raise AttributeError(name)
        return type(name, (), {})


# The oracle venv lacks opentelemetry-sdk, which the New Relic region table import needs.
class SdkStubs(MetaPathFinder, Loader):
    @override
    def find_spec(
        self,
        fullname: str,
        path: Sequence[str] | None,
        target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        if fullname.startswith(SDK_PREFIXES):
            return ModuleSpec(fullname, self, is_package=True)
        return None

    @override
    def create_module(self, spec: ModuleSpec) -> ModuleType:
        return StubModule(spec.name)

    @override
    def exec_module(self, module: ModuleType) -> None:
        return None


if find_spec("opentelemetry.sdk") is None:
    sys.meta_path.append(SdkStubs())

namespace = cast(dict[str, object], vars(validation))
arize_error = cast(NamedCheck, namespace["_arize_sampling_rate_error"])
environment_error = cast(VarsCheck, namespace["_langfuse_environment_error"])
span_scope_error = cast(NamedCheck, namespace["_langfuse_span_scope_error"])
newrelic_error = cast(VarsCheck, namespace["_newrelic_config_error"])


class Arguments(NamedTuple):
    name: str | None
    callback_vars: dict[str, str]
    stored: list[dict[str, str]]
    metadata: dict[str, Json] | None


FUNCTIONS: dict[str, Callable[[Arguments], str | None]] = {
    "callback_config_error": lambda a: validation.callback_config_error(a.name, a.callback_vars),
    "arize_sampling_rate_error": lambda a: arize_error(a.name, a.callback_vars),
    "langfuse_environment_error": lambda a: environment_error(a.callback_vars),
    "langfuse_span_scope_error": lambda a: span_scope_error(a.name, a.callback_vars),
    "newrelic_config_error": lambda a: newrelic_error(a.callback_vars),
    "cross_entry_family_error": lambda a: validation.cross_entry_family_error(
        a.callback_vars, a.stored
    ),
    "conflicting_span_scope_error": lambda a: validation.conflicting_span_scope_error(
        a.callback_vars, a.stored
    ),
    "logging_metadata_config_error": lambda a: validation.logging_metadata_config_error(a.metadata),
}


def string_map(value: Json, name: str) -> dict[str, str]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    texts: dict[str, str] = {}
    for key, item in value.items():
        if not isinstance(item, str):
            raise InputError(f"{name}.{key} must be a string")
        texts[key] = item
    return texts


def error_facts(message: str) -> Json:
    for kind, pattern in ERROR_KINDS:
        found = re.match(pattern, message)
        if found is not None:
            groups = found.groupdict()
            return {
                "error": {
                    "kind": kind,
                    "var": groups.get("var"),
                    "family": groups.get("family"),
                    "message": message,
                }
            }
    raise InputError(f"unclassified validator message: {message}")


@contextmanager
def otel_v2(*, enabled: bool) -> Generator[None]:
    saved = os.environ.pop("LITELLM_OTEL_V2", None)
    os.environ["LITELLM_OTEL_V2"] = "true" if enabled else "false"
    is_otel_v2_enabled.cache_clear()
    try:
        yield
    finally:
        os.environ.pop("LITELLM_OTEL_V2", None)
        if saved is not None:
            os.environ["LITELLM_OTEL_V2"] = saved
        is_otel_v2_enabled.cache_clear()


def check(step: dict[str, Json]) -> str | None:
    name = step.get("callback_name")
    if name is not None and not isinstance(name, str):
        raise InputError("callback_name must be a string or null")
    callback_vars = string_map(step.get("callback_vars", {}), "callback_vars")
    stored_value = step.get("stored", [])
    if not isinstance(stored_value, list):
        raise InputError("stored must be a list")
    stored = [string_map(entry, "stored") for entry in stored_value]
    metadata = step.get("metadata")
    if metadata is not None and not isinstance(metadata, dict):
        raise InputError("metadata must be an object or null")
    function = step.get("function")
    if not isinstance(function, str) or function not in FUNCTIONS:
        raise InputError(f"unknown callback_validation function: {function}")
    return FUNCTIONS[function](Arguments(name, callback_vars, stored, metadata))


def run_step(step: Json) -> Json:
    if not isinstance(step, dict) or not set(step) <= STEP_FIELDS:
        raise InputError(f"each step is an object with fields from {sorted(STEP_FIELDS)}")
    enabled = step.get("otel_v2", False)
    if not isinstance(enabled, bool):
        raise InputError("otel_v2 must be a boolean")
    with otel_v2(enabled=enabled):
        message = check(step)
    return {"ok": True} if message is None else error_facts(message)


def run(input_value: Json) -> Json:
    if not isinstance(input_value, dict) or set(input_value) != {"steps"}:
        raise InputError("input fields must be ['steps']")
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise InputError("steps must be a list")
    return [run_step(step) for step in steps]
