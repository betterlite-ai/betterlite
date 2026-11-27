import importlib.metadata

import betterlite

import smoke_wheel

DISTRIBUTION = "betterlite"


def distribution_modules() -> set[str]:
    files = importlib.metadata.distribution(DISTRIBUTION).files
    if files is None:
        raise LookupError(f"{DISTRIBUTION} has no installed file list")
    paths = [file.as_posix() for file in files]
    modules = [
        path[: -len(".py")].replace("/", ".")
        for path in paths
        if path.startswith(f"{DISTRIBUTION}/") and path.endswith(".py")
    ]
    return {module.removesuffix(".__init__") for module in modules}


def test_module_list() -> None:
    modules = smoke_wheel.installed_modules()
    assert set(modules) == distribution_modules()
    assert "betterlite.anthropic_interface.exceptions.exception_mapping_utils" in modules
    assert "betterlite.types.proxy.guardrails.guardrail_hooks.hiddenlayer" in modules


def test_main_passes() -> None:
    installed_version = betterlite.__version__
    assert smoke_wheel.main([installed_version]) == 0
