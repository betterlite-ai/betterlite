import argparse
import shutil
from pathlib import Path

GIB = 1024**3


def capacity(free: int, current: int) -> int:
    return max(current, min(24 * GIB, current + max(0, free - 64 * GIB)))


class Arguments(argparse.Namespace):
    disk: Path
    cache: Path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Keep a fixed disk reserve when sizing the compiler cache at server startup."
    )
    parser.add_argument("disk", type=Path)
    parser.add_argument("cache", type=Path)
    args = parser.parse_args(namespace=Arguments())
    current = sum(path.stat().st_size for path in args.cache.glob("**/*") if path.is_file())
    print(capacity(shutil.disk_usage(args.disk).free, current))
