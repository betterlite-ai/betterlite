import sys
from collections.abc import Callable
from importlib import import_module
from importlib.metadata import distribution
from types import ModuleType

import betterlite
from betterlite import _native, _version

DISTRIBUTION = "betterlite"
PACKAGE_PREFIX = f"{DISTRIBUTION}/"
PYTHON_SUFFIX = ".py"
INIT_SUFFIX = ".__init__"
USAGE = "usage: smoke_wheel.py <expected version>\n"

Importer = Callable[[str], ModuleType]


def module_names(paths: list[str]) -> list[str]:
    names: list[str] = []
    for path in paths:
        if not path.startswith(PACKAGE_PREFIX) or not path.endswith(PYTHON_SUFFIX):
            continue
        name = path[: -len(PYTHON_SUFFIX)].replace("/", ".")
        names.append(name.removesuffix(INIT_SUFFIX))
    return sorted(names)


def installed_modules() -> list[str]:
    files = distribution(DISTRIBUTION).files
    if files is None:
        raise LookupError(f"{DISTRIBUTION} has no installed file list")
    return module_names([str(file) for file in files])


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


def main(argv: list[str], importer: Importer = import_module) -> int:
    if len(argv) != 1:
        sys.stderr.write(USAGE)
        return 2
    for name in installed_modules():
        importer(name)
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
