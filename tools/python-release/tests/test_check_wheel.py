import zipfile
from dataclasses import dataclass, replace
from pathlib import Path

import pytest

import check_wheel

VERSION = "0.1.0"
PLATFORM = "manylinux_2_28_x86_64"
MANYLINUX_X86 = r"^manylinux_2_28_x86_64$"
SUMMARY = "LiteLLM's SDK and proxy with Rust inside"
DESCRIPTION = "The wheel ships the SDK and the native core."
LICENSE_TEXT = b"MIT License\n"
REQUIRES_DIST = ("python-dotenv==1.2.4", "typing-extensions==4.16.0")
FIVE_PLATFORMS = [
    ("manylinux_2_28_x86_64", r"^manylinux_2_28_x86_64$"),
    ("manylinux_2_28_aarch64", r"^manylinux_2_28_aarch64$"),
    ("macosx_11_0_arm64", r"^macosx_[0-9]+_[0-9]+_arm64$"),
    ("macosx_10_12_x86_64", r"^macosx_[0-9]+_[0-9]+_x86_64$"),
    ("win_amd64", r"^win_amd64$"),
]


@dataclass(frozen=True)
class WheelSpec:
    version: str = VERSION
    platform_tag: str = PLATFORM
    python_tag: str = "cp310"
    abi_tag: str = "abi3"
    metadata_name: str = "betterlite"
    metadata_version: str = VERSION
    requires_python: str = ">=3.10"
    license_expression: str = "MIT"
    summary: str = SUMMARY
    description: str = DESCRIPTION
    requires_dist: tuple[str, ...] = REQUIRES_DIST
    native: tuple[str, ...] | None = None
    init: bool = True
    py_typed: bool = True
    license_file: bool = True
    extra_entries: tuple[str, ...] = ()
    omit_from_record: tuple[str, ...] = ()
    omitted_headers: tuple[str, ...] = ()
    extra_headers: tuple[str, ...] = ()
    omit_metadata: bool = False
    omit_record: bool = False
    metadata_bytes: bytes | None = None
    record_bytes: bytes | None = None


BASE_WHEEL = WheelSpec()


def native_entries(spec: WheelSpec) -> tuple[str, ...]:
    if spec.native is not None:
        return spec.native
    if spec.platform_tag.startswith("win"):
        return ("betterlite/_native.pyd",)
    return ("betterlite/_native.abi3.so",)


def metadata_text(spec: WheelSpec) -> str:
    lines = [
        "Metadata-Version: 2.4",
        f"Name: {spec.metadata_name}",
        f"Version: {spec.metadata_version}",
        f"Requires-Python: {spec.requires_python}",
        f"License-Expression: {spec.license_expression}",
        f"Summary: {spec.summary}",
        *(f"Requires-Dist: {value}" for value in spec.requires_dist),
        *spec.extra_headers,
    ]
    kept = [line for line in lines if line.split(":", 1)[0] not in spec.omitted_headers]
    return "\n".join(kept) + "\n\n" + spec.description + "\n"


def wheel_text(spec: WheelSpec) -> str:
    tag = f"{spec.python_tag}-{spec.abi_tag}-{spec.platform_tag}"
    return f"Wheel-Version: 1.0\nGenerator: tests\nRoot-Is-Purelib: false\nTag: {tag}\n"


def wheel_entries(spec: WheelSpec, dist_info: str, record: str) -> list[str]:
    names: list[str] = []
    if spec.init:
        names.append("betterlite/__init__.py")
    if spec.py_typed:
        names.append("betterlite/py.typed")
    names.extend(native_entries(spec))
    names.extend(spec.extra_entries)
    if not spec.omit_metadata:
        names.append(f"{dist_info}/METADATA")
    names.append(f"{dist_info}/WHEEL")
    if spec.license_file:
        names.append(f"{dist_info}/licenses/LICENSE")
    if not spec.omit_record:
        names.append(record)
    return names


def build_wheel(directory: Path, spec: WheelSpec) -> Path:
    dist_info = f"betterlite-{spec.version}.dist-info"
    record = f"{dist_info}/RECORD"
    names = wheel_entries(spec, dist_info, record)
    listed = [name for name in names if name != record and name not in spec.omit_from_record]
    metadata_payload = metadata_text(spec).encode()
    if spec.metadata_bytes is not None:
        metadata_payload = spec.metadata_bytes
    record_payload = "".join(f"{name},,\n" for name in listed).encode()
    if spec.record_bytes is not None:
        record_payload = spec.record_bytes
    payloads = {
        f"{dist_info}/METADATA": metadata_payload,
        f"{dist_info}/WHEEL": wheel_text(spec).encode(),
        record: record_payload,
        f"{dist_info}/licenses/LICENSE": LICENSE_TEXT,
    }
    stem = f"betterlite-{spec.version}-{spec.python_tag}-{spec.abi_tag}-{spec.platform_tag}"
    wheel = directory / f"{stem}.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        for name in names:
            if name in payloads:
                archive.writestr(name, payloads[name])
            else:
                archive.writestr(name, b"")
    return wheel


def run_checker(
    capsys: pytest.CaptureFixture[str], wheel: Path, pattern: str, *extra: str
) -> tuple[int, list[str]]:
    code = check_wheel.main([str(wheel), "--platform", pattern, *extra])
    captured = capsys.readouterr()
    return code, captured.out.splitlines()


@pytest.mark.parametrize(("platform_tag", "pattern"), FIVE_PLATFORMS)
def test_accepts_each_platform(
    capsys: pytest.CaptureFixture[str], tmp_path: Path, platform_tag: str, pattern: str
) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, platform_tag=platform_tag))
    code, lines = run_checker(capsys, wheel, pattern, "--tag", f"python-v{VERSION}")
    assert code == 0
    assert lines == []


def test_rejects_wrong_python_tag(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, python_tag="cp311"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "python tag is cp311" in lines[0]


def test_rejects_wrong_abi_tag(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, abi_tag="none"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "abi tag is none" in lines[0]


def test_rejects_platform_pattern(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, platform_tag=PLATFORM + ".linux_x86_64"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "platform tag linux_x86_64 does not match" in lines[0]


def test_rejects_missing_native(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, native=()))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "found 0 native modules" in lines[0]


def test_rejects_double_native(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    both = ("betterlite/_native.abi3.so", "betterlite/_native.pyd")
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, native=both))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "found 2 native modules" in lines[0]


def test_rejects_missing_init(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, init=False))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite/__init__.py is missing" in lines[0]


def test_rejects_missing_py_typed(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, py_typed=False))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite/py.typed is missing" in lines[0]


def test_rejects_tests_entry(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, extra_entries=("tests/test_wheel.py",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "tests/test_wheel.py is not allowed" in lines[0]


def test_rejects_pyc_entry(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, extra_entries=("betterlite/_version.pyc",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite/_version.pyc is not allowed" in lines[0]


def test_rejects_wrong_name(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, metadata_name="betterlite-core"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Name is betterlite-core, expected betterlite" in lines[0]


def test_rejects_wrong_version(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, metadata_version="9.9.9"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Version is 9.9.9, expected 0.1.0" in lines[0]


def test_rejects_requires_python(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, requires_python=">=3.11"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Requires-Python is >=3.11, expected >=3.10" in lines[0]


def test_rejects_wrong_license(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, license_expression="Apache-2.0"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: License-Expression is Apache-2.0, expected MIT" in lines[0]


def test_rejects_empty_summary(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, summary=""))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Summary is empty" in lines[0]


def test_rejects_empty_description(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, description=""))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Description is empty" in lines[0]


def test_rejects_extra_requires(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    dist = ("python-dotenv==1.2.4", "requests==2.32.5")
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, requires_dist=dist))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Requires-Dist requests is not allowed" in lines[0]


def test_rejects_missing_license(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, license_file=False))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "licenses/LICENSE is missing" in lines[0]


def test_rejects_record_miss(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, omit_from_record=("betterlite/py.typed",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "RECORD does not list betterlite/py.typed" in lines[0]


def test_rejects_wrong_tag(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, BASE_WHEEL)
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86, "--tag", "python-v9.9.9")
    assert code == 1
    assert len(lines) == 1
    assert "tag is python-v9.9.9, expected python-v0.1.0" in lines[0]


def test_reports_every_problem(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, python_tag="cp311", abi_tag="none"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 2
    assert "python tag is cp311" in lines[0]
    assert "abi tag is none" in lines[1]


def test_usage_without_wheel(capsys: pytest.CaptureFixture[str]) -> None:
    assert check_wheel.main([]) == 2
    assert "usage: check_wheel.py" in capsys.readouterr().err


def test_missing_option_value(capsys: pytest.CaptureFixture[str]) -> None:
    assert check_wheel.main(["--platform"]) == 2
    assert "--platform needs a value" in capsys.readouterr().err


def test_unknown_option(capsys: pytest.CaptureFixture[str]) -> None:
    assert check_wheel.main(["--platform", "x", "--force"]) == 2
    assert "unknown option --force" in capsys.readouterr().err


def test_rejects_non_whl(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    path = tmp_path / "betterlite-0.1.0-cp310-abi3-manylinux_2_28_x86_64.zip"
    code, lines = run_checker(capsys, path, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "not a .whl file" in lines[0]


def test_rejects_malformed_name(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    path = tmp_path / "betterlite-0.1.0-cp310-manylinux_2_28_x86_64.whl"
    code, lines = run_checker(capsys, path, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "the name is not betterlite-<version>-cp310-abi3-<platform>" in lines[0]


def test_rejects_foreign_name(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, BASE_WHEEL)
    foreign = wheel.rename(tmp_path / "litellm-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl")
    code, lines = run_checker(capsys, foreign, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "the distribution is litellm, expected betterlite" in lines[0]


def test_rejects_missing_header(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, omitted_headers=("Requires-Python",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Requires-Python is missing" in lines[0]


def test_rejects_duplicate_header(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, extra_headers=("Name: betterlite",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: Name appears 2 times" in lines[0]


def test_rejects_odd_requires(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, requires_dist=("==1.0",)))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "METADATA: cannot read Requires-Dist ==1.0" in lines[0]


def test_rejects_missing_metadata(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, omit_metadata=True))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite-0.1.0.dist-info/METADATA is missing" in lines[0]


def test_rejects_non_utf8_metadata(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, metadata_bytes=b"\xff"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite-0.1.0.dist-info/METADATA is not UTF-8" in lines[0]


def test_rejects_missing_record(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, omit_record=True))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite-0.1.0.dist-info/RECORD is missing" in lines[0]


def test_rejects_non_utf8_record(capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    wheel = build_wheel(tmp_path, replace(BASE_WHEEL, record_bytes=b"\xff"))
    code, lines = run_checker(capsys, wheel, MANYLINUX_X86)
    assert code == 1
    assert len(lines) == 1
    assert "betterlite-0.1.0.dist-info/RECORD is not UTF-8" in lines[0]
