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


class FixedPid:
    def __init__(self, pid: int) -> None:
        self.pid = pid

    def __call__(self) -> int:
        return self.pid


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


def apply_counter(metric: Counter, update: UpdateSpec) -> None:
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    match update["action"]:
        case "inc":
            target.inc(update["value"])
        case "remove":
            metric.remove(*update["labels"])
        case "set" | "observe":
            raise ValueError("a counter takes inc or remove")


def apply_gauge(metric: Gauge, update: UpdateSpec) -> None:
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    match update["action"]:
        case "inc":
            target.inc(update["value"])
        case "set":
            target.set(update["value"])
        case "remove":
            metric.remove(*update["labels"])
        case "observe":
            raise ValueError("a gauge takes inc, set or remove")


def apply_histogram(metric: Histogram, update: UpdateSpec) -> None:
    target = metric if not update["labels"] else metric.labels(*update["labels"])
    match update["action"]:
        case "observe":
            target.observe(update["value"])
        case "remove":
            metric.remove(*update["labels"])
        case "inc" | "set":
            raise ValueError("a histogram takes observe or remove")


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
    if input_value["call"] == "files":
        return files_answer(folder)
    return scrape_answer(folder, input_value["accept"])


def read_entries(path: Path) -> list[tuple[str, float, float]]:
    module = prometheus_module()
    mmap = module.MmapedDict(str(path))
    entries = list(mmap.read_all_values())
    mmap.close()
    return entries


def files_answer(directory: Path) -> FilesAnswer:
    files: list[FileOut] = []
    for path in sorted(directory.glob("*.db")):
        data = path.read_bytes()
        used = int.from_bytes(data[:4], "little", signed=True)
        entries: list[EntryOut] = []
        for key, value, timestamp in read_entries(path):
            entries.append(
                {"key": KEY.validate_json(key), "value": value, "timestamp": timestamp}
            )
        files.append(
            {
                "name": path.name,
                "length": path.stat().st_size,
                "used": max(used, 0),
                "entries": entries,
            }
        )
    return {"files": files}


def used_hex(path: Path) -> str:
    data = path.read_bytes()
    used = int.from_bytes(data[:4], "little", signed=True)
    return data[: max(used, 0)].hex()


def scrape_answer(directory: Path, accept: str) -> ScrapeAnswer:
    module = prometheus_module()
    registry = CollectorRegistry()
    _collector = module.MultiProcessCollector(registry, str(directory))
    scrape = parse_scrape_request(accept, "", ())
    body = render_scrape(registry, scrape)
    files = [used_hex(path) for path in sorted(directory.glob("*.db"))]
    return {"files": files, "blocks": blocks(body.decode("utf-8"))}


def blocks(text: str) -> list[str]:
    sections: list[tuple[str, list[str]]] = []
    for line in text.splitlines():
        marker = ""
        if line.startswith("# HELP "):
            marker = line.split(" ")[2]
        if line == "# EOF":
            marker = "# EOF"
        if marker:
            sections.append((marker, []))
        if sections:
            sections[-1][1].append(line)
    for _, lines in sections:
        lines.sort()
    indexed: list[tuple[str, int]] = [
        (marker, index) for index, (marker, _lines) in enumerate(sections)
    ]
    indexed.sort()
    return ["\n".join(sections[index][1]) for _, index in indexed]


def run(input_value: Input) -> FilesAnswer | ScrapeAnswer:
    previous_directory = os.environ.pop("PROMETHEUS_MULTIPROC_DIR", None)
    previous_value_class = values.ValueClass
    try:
        values.ValueClass = values.MultiProcessValue(FixedPid(input_value["pid"]))
        with tempfile.TemporaryDirectory() as folder:
            os.environ["PROMETHEUS_MULTIPROC_DIR"] = folder
            answer = run_in_folder(input_value, Path(folder))
    finally:
        values.ValueClass = previous_value_class
        os.environ.pop("PROMETHEUS_MULTIPROC_DIR", None)
        if previous_directory is not None:
            os.environ["PROMETHEUS_MULTIPROC_DIR"] = previous_directory
    return answer
