import importlib
import os
import tempfile
from collections.abc import Callable, Iterable
from pathlib import Path
from types import ModuleType
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.integrations.prometheus_metrics_endpoint import (
    parse_scrape_request,
    render_scrape,
)
from prometheus_client import CollectorRegistry, Counter, Gauge, Histogram, values
from prometheus_client.metrics_core import Metric as CoreMetric
from prometheus_client.mmap_dict import MmapedDict
from prometheus_client.multiprocess import MultiProcessCollector
from pydantic import TypeAdapter

KEY = TypeAdapter(tuple[str, str, dict[str, str], str])


class MetricSpec(TypedDict):
    kind: Literal["counter", "gauge", "histogram"]
    name: str
    help: str
    labels: list[str]
    buckets: list[float]
    mode: Literal["all", "livesum", "livemax"]


class UpdateSpec(TypedDict):
    metric: int
    labels: list[str]
    action: Literal["inc", "set", "observe", "remove"]
    value: float


class Input(TypedDict):
    pid: int
    metrics: list[MetricSpec]
    updates: list[UpdateSpec]
    call: Literal["files", "scrape"]
    accept: str


class EntryOut(TypedDict):
    key: tuple[str, str, dict[str, str], str]
    value: float
    timestamp: float


class FileOut(TypedDict):
    name: str
    length: int
    used: int
    entries: list[EntryOut]


class FilesAnswer(TypedDict):
    files: list[FileOut]


class ScrapeAnswer(TypedDict):
    files: list[str]
    blocks: list[str]


class Mmap(Protocol):
    def read_all_values(self) -> Iterable[tuple[str, float, float]]: ...

    def close(self) -> None: ...


class ProcessCollector(Protocol):
    def collect(self) -> Iterable[CoreMetric]: ...


class PrometheusModule(ModuleType):
    MmapedDict: Callable[[str], Mmap]
    MultiProcessCollector: Callable[[CollectorRegistry, str], ProcessCollector]


Metric: TypeAlias = Counter | Gauge | Histogram


def prometheus_module() -> PrometheusModule:
    module = PrometheusModule("prometheus_client")
    module.MmapedDict = MmapedDict
    module.MultiProcessCollector = MultiProcessCollector
    return module


def build_metric(spec: MetricSpec) -> Metric:
    if spec["kind"] == "counter":
        return Counter(
            spec["name"], spec["help"], labelnames=spec["labels"], registry=None
        )
    if spec["kind"] == "gauge":
        return Gauge(
            spec["name"],
            spec["help"],
            labelnames=spec["labels"],
            registry=None,
            multiprocess_mode=spec["mode"],
        )
    if spec["kind"] == "histogram":
        return Histogram(
            spec["name"],
            spec["help"],
            labelnames=spec["labels"],
            buckets=spec["buckets"],
            registry=None,
        )
    raise ValueError(f"unknown metric kind {spec['kind']}")


def require_action(action: str, allowed: tuple[str, ...]) -> None:
    if action not in allowed:
        raise ValueError(f"a metric of this kind takes {', '.join(allowed)}")


def apply_counter(metric: Counter, update: UpdateSpec) -> None:
    if update["action"] == "remove":
        metric.remove(*update["labels"])
        return
    require_action(update["action"], ("inc",))
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    target.inc(update["value"])


def apply_gauge(metric: Gauge, update: UpdateSpec) -> None:
    if update["action"] == "remove":
        metric.remove(*update["labels"])
        return
    require_action(update["action"], ("inc", "set"))
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    if update["action"] == "inc":
        target.inc(update["value"])
    else:
        target.set(update["value"])


def apply_histogram(metric: Histogram, update: UpdateSpec) -> None:
    if update["action"] == "remove":
        metric.remove(*update["labels"])
        return
    require_action(update["action"], ("observe",))
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    target.observe(update["value"])


def apply_update(metric: Metric, update: UpdateSpec) -> None:
    match metric:
        case Counter():
            apply_counter(metric, update)
        case Gauge():
            apply_gauge(metric, update)
        case Histogram():
            apply_histogram(metric, update)


def run_in_folder(input_value: Input, folder: Path) -> FilesAnswer | ScrapeAnswer:
    metrics: list[Metric] = [build_metric(spec) for spec in input_value["metrics"]]
    for update in input_value["updates"]:
        apply_update(metrics[update["metric"]], update)
    rename_to_pid(folder, input_value["pid"])
    if input_value["call"] == "files":
        return {"files": [file_out(path) for path in sorted(folder.glob("*.db"))]}
    return scrape_answer(folder, input_value["accept"])


# the files carry this process's pid; the case's pid names the files both sides compare
def rename_to_pid(folder: Path, pid: int) -> None:
    for path in folder.glob("*.db"):
        stem = path.name.rsplit("_", 1)[0]
        path.rename(folder / f"{stem}_{pid}.db")


def read_entries(path: Path) -> list[tuple[str, float, float]]:
    module = prometheus_module()
    mmap = module.MmapedDict(str(path))
    entries = list(mmap.read_all_values())
    mmap.close()
    return entries


def used_length(data: bytes) -> int:
    used = int.from_bytes(data[:4], "little", signed=True)
    return max(used, 0)


def file_out(path: Path) -> FileOut:
    data = path.read_bytes()
    entries: list[EntryOut] = [
        {"key": KEY.validate_json(key), "value": value, "timestamp": timestamp}
        for key, value, timestamp in read_entries(path)
    ]
    return {
        "name": path.name,
        "length": len(data),
        "used": used_length(data),
        "entries": entries,
    }


def used_hex(path: Path) -> str:
    data = path.read_bytes()
    return data[: used_length(data)].hex()


def scrape_answer(directory: Path, accept: str) -> ScrapeAnswer:
    module = prometheus_module()
    registry = CollectorRegistry()
    _collector = module.MultiProcessCollector(registry, str(directory))
    scrape = parse_scrape_request(accept, "", ())
    body = render_scrape(registry, scrape)
    files = [used_hex(path) for path in sorted(directory.glob("*.db"))]
    return {"files": files, "blocks": blocks(body.decode("utf-8"))}


def blocks(text: str) -> list[str]:
    grouped: dict[str, list[str]] = {}
    marker = ""
    for line in text.splitlines():
        if line.startswith("# HELP "):
            marker = line.split(" ")[2]
        elif line == "# EOF":
            marker = "# EOF"
        if marker:
            grouped.setdefault(marker, []).append(line)
    return ["\n".join(sorted(grouped[name])) for name in sorted(grouped)]


def run(input_value: Input) -> FilesAnswer | ScrapeAnswer:
    previous = os.environ.pop("PROMETHEUS_MULTIPROC_DIR", None)
    try:
        with tempfile.TemporaryDirectory() as folder:
            os.environ["PROMETHEUS_MULTIPROC_DIR"] = folder
            # values.py picks its value class at import, so reload it with the folder set
            importlib.reload(values)
            answer = run_in_folder(input_value, Path(folder))
    finally:
        if previous is None:
            os.environ.pop("PROMETHEUS_MULTIPROC_DIR", None)
        else:
            os.environ["PROMETHEUS_MULTIPROC_DIR"] = previous
        importlib.reload(values)
    return answer
