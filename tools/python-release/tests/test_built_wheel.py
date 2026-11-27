import os
import tomllib
import zipfile
from pathlib import Path
from typing import TypedDict

import betterlite
from betterlite import _native, _version
from betterlite.types.utils import JsonValue as Json
from pydantic import TypeAdapter

import check_wheel

ROOT = Path(__file__).resolve().parents[3]
PLATFORM = r"^(linux|manylinux)_[a-z0-9_]+$"


class CargoPackage(TypedDict):
    version: str


class CargoWorkspace(TypedDict):
    package: CargoPackage


class CargoToml(TypedDict):
    workspace: CargoWorkspace


def crate_version() -> str:
    parsed: dict[str, Json] = tomllib.loads((ROOT / "Cargo.toml").read_text())
    return TypeAdapter(CargoToml).validate_python(parsed)["workspace"]["package"]["version"]


def built_wheel() -> Path:
    wheels = sorted(Path(os.environ["BETTERLITE_WHEEL_DIR"]).glob("*.whl"))
    assert len(wheels) == 1
    return wheels[0]


def wheel_metadata() -> str:
    with zipfile.ZipFile(built_wheel()) as archive:
        names = [name for name in archive.namelist() if name.endswith(".dist-info/METADATA")]
        assert len(names) == 1
        return archive.read(names[0]).decode("utf-8")


def test_checker_accepts_wheel() -> None:
    tag = f"python-v{crate_version()}"
    code = check_wheel.main([str(built_wheel()), "--platform", PLATFORM, "--tag", tag])
    assert code == 0


def test_metadata_headers() -> None:
    text = wheel_metadata()
    assert "Author: Max Azatian\n" in text
    assert "Classifier: License :: OSI Approved :: MIT License\n" in text
    assert "Classifier: Operating System :: MacOS\n" in text
    assert "Classifier: Operating System :: Microsoft :: Windows\n" in text
    assert "Classifier: Operating System :: POSIX :: Linux\n" in text
    assert "Classifier: Programming Language :: Rust\n" in text
    assert "Project-URL: Repository, https://github.com/betterlite-ai/betterlite\n" in text


def test_versions_match_crate() -> None:
    version = crate_version()
    assert betterlite.__version__ == version
    assert _version.version == version
    assert _native.core_version() == version
