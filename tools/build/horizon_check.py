"""Stage the native adapter inside its pinned Horizon workspace for a real API check.

Horizon patches wgpu-hal/naga locally; Cargo does not inherit a git dependency's
workspace patches. The launcher also builds native mods against its own source
workspace, so a standalone dependency-only check is not equivalent.
"""

import argparse
import shutil
import subprocess
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("upstream", type=Path)
    upstream = parser.parse_args().upstream.resolve()
    bundle = REPO / "horizon/native"
    manifest = tomllib.loads((bundle / "horizon-mixin.toml").read_text(encoding="utf-8"))
    pin = manifest["compatibility"]["horizon_revision"]
    revision = subprocess.check_output(["git", "-C", str(upstream), "rev-parse", "HEAD"], text=True).strip()
    if revision != pin:
        raise SystemExit(f"Horizon revision mismatch: expected {pin}, got {revision}")
    source = bundle / "api"
    api = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))
    dependency = api["dependencies"]["horizon-client"]
    if dependency.get("rev") != pin:
        raise SystemExit("Native API dependency and bundle revision disagree")
    destination = upstream / "crates/halfcraft-horizon"
    if destination.exists():
        raise SystemExit(f"Refusing to overwrite {destination}")
    workspace = upstream / "Cargo.toml"
    text = workspace.read_text(encoding="utf-8")
    marker = "members = [\n"
    if text.count(marker) != 1:
        raise SystemExit("Unrecognized upstream workspace member declaration")
    cargo = (source / "Cargo.toml").read_text(encoding="utf-8")
    expected = f'horizon-client = {{ git = "{dependency["git"]}", rev = "{pin}" }}'
    if cargo.count(expected) != 1:
        raise SystemExit("Unrecognized native API dependency declaration")
    shutil.copytree(source, destination, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    (destination / "Cargo.toml").write_text(cargo.replace(expected, 'horizon-client = { path = "../horizon-client" }'), encoding="utf-8")
    workspace.write_text(text.replace(marker, marker + '    "crates/halfcraft-horizon",\n', 1), encoding="utf-8")
    print(f"Staged {api['package']['name']} against Horizon {pin}, preserving its workspace patches and lockfile")


if __name__ == "__main__":
    main()
