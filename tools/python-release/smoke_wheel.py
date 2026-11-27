import sys
from importlib import import_module

import betterlite
from betterlite import _native, _version


def version_failures(expected: str) -> list[str]:
    failures: list[str] = []
    if betterlite.__version__ != expected:
        failures.append(f"betterlite.__version__ is {betterlite.__version__}, expected {expected}")
    if _version.version != expected:
        failures.append(f"betterlite._version.version is {_version.version}, expected {expected}")
    core = _native.core_version()
    if core != expected:
        failures.append(f"betterlite._native.core_version() is {core}, expected {expected}")
    return failures


def litellm_failure() -> str | None:
    try:
        module = import_module("litellm")
    except ModuleNotFoundError as error:
        if error.name != "litellm":
            raise
        return None
    return f"the environment imports litellm as {module.__name__}"


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        sys.stderr.write("usage: smoke_wheel.py <expected version>\n")
        return 2
    failures = version_failures(argv[0])
    litellm_problem = litellm_failure()
    if litellm_problem is not None:
        failures.append(litellm_problem)
    for failure in failures:
        sys.stderr.write(f"{failure}\n")
    if failures:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
