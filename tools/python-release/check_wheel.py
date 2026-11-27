import re
import sys
import zipfile
from dataclasses import dataclass
from email.parser import Parser
from pathlib import Path

DISTRIBUTION = "betterlite"
EXPECTED_PYTHON_TAG = "cp310"
EXPECTED_ABI_TAG = "abi3"
EXPECTED_REQUIRES_PYTHON = ">=3.10"
EXPECTED_LICENSE = "MIT"
EXPECTED_NAME = f"{DISTRIBUTION}-<version>-{EXPECTED_PYTHON_TAG}-{EXPECTED_ABI_TAG}-<platform>"
NAME_PARTS = 5
SO_MODULE = "betterlite/_native.abi3.so"
PYD_MODULE = "betterlite/_native.pyd"
NATIVE_MODULES = (SO_MODULE, PYD_MODULE)
REQUIRED_MODULES = ("betterlite/__init__.py", "betterlite/py.typed")
ALLOWED_REQUIRES_DIST = ("httpx2", "pydantic", "python-dotenv", "typing-extensions")
HEADER_NAMES = (
    "Name",
    "Version",
    "Requires-Python",
    "License-Expression",
    "Summary",
    "Requires-Dist",
)
USAGE = "usage: check_wheel.py <wheel> --platform <regex> [--tag <git tag>]\n"


@dataclass(frozen=True)
class Arguments:
    wheel: Path
    platform: str
    tag: str | None


@dataclass(frozen=True)
class WheelFile:
    path: Path
    version: str
    platform: str


def parse_arguments(argv: list[str]) -> Arguments | None:
    platform: str | None = None
    tag: str | None = None
    wheels: list[str] = []
    index = 0
    while index < len(argv):
        argument = argv[index]
        if argument in ("--platform", "--tag"):
            if index + 1 == len(argv):
                sys.stderr.write(f"{argument} needs a value\n")
                return None
            value = argv[index + 1]
            if argument == "--platform":
                platform = value
            else:
                tag = value
            index += 2
            continue
        if argument.startswith("-"):
            sys.stderr.write(f"unknown option {argument}\n")
            return None
        wheels.append(argument)
        index += 1
    if len(wheels) != 1 or platform is None:
        sys.stderr.write(USAGE)
        return None
    return Arguments(wheel=Path(wheels[0]), platform=platform, tag=tag)


def read_wheel_name(path: Path, problems: list[str]) -> WheelFile | None:
    if path.suffix != ".whl":
        problems.append(f"{path.name}: not a .whl file")
        return None
    parts = path.stem.split("-")
    if len(parts) != NAME_PARTS:
        problems.append(f"{path.name}: the name is not {EXPECTED_NAME}")
        return None
    distribution, version, python_tag, abi_tag, platform = parts
    if distribution != DISTRIBUTION:
        problems.append(f"{path.name}: the distribution is {distribution}, expected {DISTRIBUTION}")
    if python_tag != EXPECTED_PYTHON_TAG:
        problems.append(f"{path.name}: python tag is {python_tag}, expected {EXPECTED_PYTHON_TAG}")
    if abi_tag != EXPECTED_ABI_TAG:
        problems.append(f"{path.name}: abi tag is {abi_tag}, expected {EXPECTED_ABI_TAG}")
    return WheelFile(path=path, version=version, platform=platform)


def check_platform(wheel: WheelFile, pattern: str, problems: list[str]) -> None:
    pieces = wheel.platform.split(".")
    problems.extend(
        f"{wheel.path.name}: platform tag {piece} does not match {pattern}"
        for piece in pieces
        if re.fullmatch(pattern, piece) is None
    )


def check_entries(wheel: WheelFile, archive: zipfile.ZipFile, problems: list[str]) -> None:
    entries = archive.namelist()
    present = set(entries)
    problems.extend(
        f"{wheel.path.name}: {required} is missing"
        for required in REQUIRED_MODULES
        if required not in present
    )
    native = PYD_MODULE if wheel.platform.startswith("win") else SO_MODULE
    found = [entry for entry in entries if entry in NATIVE_MODULES]
    if found != [native]:
        problems.append(
            f"{wheel.path.name}: expected one {native}, found {len(found)} native modules"
        )
    problems.extend(
        f"{wheel.path.name}: {entry} is not allowed in a wheel"
        for entry in entries
        if entry.startswith("tests/") or entry.endswith(".pyc")
    )


def read_headers(block: str) -> dict[str, list[str]]:
    message = Parser().parsestr(block)
    headers: dict[str, list[str]] = {}
    for name in HEADER_NAMES:
        values = message.get_all(name)
        if values is not None:
            headers[name] = values
    return headers


def one_header(headers: dict[str, list[str]], name: str, problems: list[str]) -> str | None:
    values = headers.get(name)
    if values is None:
        problems.append(f"METADATA: {name} is missing")
        return None
    if len(values) != 1:
        problems.append(f"METADATA: {name} appears {len(values)} times")
        return None
    return values[0]


def check_fields(wheel: WheelFile, headers: dict[str, list[str]], problems: list[str]) -> None:
    expected = {
        "Name": DISTRIBUTION,
        "Version": wheel.version,
        "Requires-Python": EXPECTED_REQUIRES_PYTHON,
        "License-Expression": EXPECTED_LICENSE,
    }
    for name, value in expected.items():
        found = one_header(headers, name, problems)
        if found is not None and found != value:
            problems.append(f"METADATA: {name} is {found}, expected {value}")


def check_text_fields(headers: dict[str, list[str]], body: str, problems: list[str]) -> None:
    summary = one_header(headers, "Summary", problems)
    if summary is not None and summary.strip() == "":
        problems.append("METADATA: Summary is empty")
    if body.strip() == "":
        problems.append("METADATA: Description is empty")


def requirement_name(value: str) -> str | None:
    match = re.match(r"^[A-Za-z0-9][A-Za-z0-9._-]*", value)
    if match is None:
        return None
    return match.group(0).lower().replace("_", "-").replace(".", "-")


def check_requires_dist(headers: dict[str, list[str]], problems: list[str]) -> None:
    values = headers.get("Requires-Dist")
    if values is None:
        return
    for value in values:
        name = requirement_name(value)
        if name is None:
            problems.append(f"METADATA: cannot read Requires-Dist {value}")
        elif name not in ALLOWED_REQUIRES_DIST:
            problems.append(f"METADATA: Requires-Dist {name} is not allowed")


def check_metadata(wheel: WheelFile, archive: zipfile.ZipFile, problems: list[str]) -> None:
    dist_info = f"{DISTRIBUTION}-{wheel.version}.dist-info"
    metadata = f"{dist_info}/METADATA"
    try:
        raw = archive.read(metadata)
    except KeyError:
        problems.append(f"{wheel.path.name}: {metadata} is missing")
        return
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        problems.append(f"{wheel.path.name}: {metadata} is not UTF-8")
        return
    block, _, body = text.partition("\n\n")
    headers = read_headers(block)
    check_fields(wheel, headers, problems)
    check_text_fields(headers, body, problems)
    check_requires_dist(headers, problems)
    license_file = f"{dist_info}/licenses/LICENSE"
    if license_file not in archive.namelist():
        problems.append(f"{wheel.path.name}: {license_file} is missing")


def check_record(wheel: WheelFile, archive: zipfile.ZipFile, problems: list[str]) -> None:
    record = f"{DISTRIBUTION}-{wheel.version}.dist-info/RECORD"
    try:
        rows = archive.read(record).decode("utf-8").splitlines()
    except KeyError:
        problems.append(f"{wheel.path.name}: {record} is missing")
        return
    except UnicodeDecodeError:
        problems.append(f"{wheel.path.name}: {record} is not UTF-8")
        return
    listed = {row.split(",", 1)[0] for row in rows if row}
    missing = [entry for entry in archive.namelist() if entry != record and entry not in listed]
    problems.extend(f"{wheel.path.name}: RECORD does not list {entry}" for entry in missing)


def check_tag(wheel: WheelFile, tag: str | None, problems: list[str]) -> None:
    if tag is None:
        return
    expected = f"python-v{wheel.version}"
    if tag != expected:
        problems.append(f"{wheel.path.name}: tag is {tag}, expected {expected}")


def check_archive(wheel: WheelFile, problems: list[str]) -> None:
    try:
        archive = zipfile.ZipFile(wheel.path)
    except (OSError, zipfile.BadZipFile):
        problems.append(f"{wheel.path.name}: cannot open as a zip file")
        return
    with archive:
        check_entries(wheel, archive, problems)
        check_metadata(wheel, archive, problems)
        check_record(wheel, archive, problems)


def main(argv: list[str]) -> int:
    arguments = parse_arguments(argv)
    if arguments is None:
        return 2
    problems: list[str] = []
    wheel = read_wheel_name(arguments.wheel, problems)
    if wheel is not None:
        check_platform(wheel, arguments.platform, problems)
        check_archive(wheel, problems)
        check_tag(wheel, arguments.tag, problems)
    for problem in problems:
        sys.stdout.write(f"{problem}\n")
    if problems:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
