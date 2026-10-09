"""Check synthetic filtering outputs with a supplied, checksum-verified Mihomo.

The caller supplies the fixed core binary; this script downloads nothing and
never sends traffic through a proxy. Only the local controller is queried.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", required=True, type=Path)
    args = parser.parse_args()
    core = args.core.resolve(strict=True)
    repository = Path(__file__).resolve().parents[1]
    version = subprocess.run([str(core), "-v"], capture_output=True, text=True, check=True, timeout=10)
    if "v1.19.17" not in version.stdout:
        raise RuntimeError("Expected the pinned Mihomo v1.19.17 core")
    generated = subprocess.run(
        ["cargo", "run", "--locked", "--quiet", "--example", "compatibility-core-check"],
        cwd=repository, capture_output=True, text=True, encoding="utf-8", check=True, timeout=600,
    )
    fixtures = json.loads(generated.stdout)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    results = []
    with tempfile.TemporaryDirectory(prefix="compatibility-core-check-") as directory:
        root = Path(directory)
        for index, fixture in enumerate(fixtures):
            config = root / f"case-{index}.yaml"
            config.write_text(fixture["config"], encoding="utf-8")
            command = [str(core), "-d", str(root), "-f", str(config)]
            tested = subprocess.run(command + ["-t"], capture_output=True, timeout=20)
            if tested.returncode:
                raise RuntimeError(f"Core rejected synthetic case {fixture['name']}")
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            process = subprocess.Popen(
                command + ["-ext-ctl", f"127.0.0.1:{port}", "-secret", "synthetic-test-secret"],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
            )
            try:
                request = urllib.request.Request(
                    f"http://127.0.0.1:{port}/proxies/Choice",
                    headers={"Authorization": "Bearer synthetic-test-secret"},
                )
                deadline = time.monotonic() + 10
                while True:
                    if process.poll() is not None:
                        raise RuntimeError(f"Core exited during synthetic case {fixture['name']}")
                    try:
                        with opener.open(request, timeout=0.5) as response:
                            choice = json.load(response)
                        break
                    except (urllib.error.URLError, TimeoutError):
                        if time.monotonic() >= deadline:
                            raise RuntimeError(f"Core controller timed out for {fixture['name']}") from None
                        time.sleep(0.1)
                if choice["all"] != fixture["expected"] or choice["now"] != "REJECT":
                    raise RuntimeError(f"Unexpected effective members in {fixture['name']}")
                results.append({"case": fixture["name"], "config_test": "passed", "members": choice["all"], "selected": choice["now"]})
            finally:
                stop(process)
    print(json.dumps({"core": "v1.19.17", "results": results}, indent=2))


if __name__ == "__main__":
    main()
