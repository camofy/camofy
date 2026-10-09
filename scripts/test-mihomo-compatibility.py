"""Check synthetic filtering outputs with a supplied, checksum-verified Mihomo.

The caller supplies the fixed core binary; this script downloads nothing and
uses only loopback endpoints. Rule equivalence checks use a local echo server
and a local DNS stub; no production proxy or external destination is contacted.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import socketserver
import struct
import subprocess
import tempfile
import time
import threading
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


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def controller(opener, port, path):
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        headers={"Authorization": "Bearer synthetic-test-secret"},
    )
    with opener.open(request, timeout=1) as response:
        return json.load(response)


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        try:
            while data := self.request.recv(4096):
                self.request.sendall(data)
        except (OSError, TimeoutError):
            pass


class LocalDNS(socketserver.BaseRequestHandler):
    def handle(self):
        query, connection = self.request
        if len(query) < 17:
            return
        end = 12
        while end < len(query) and query[end]:
            end += query[end] + 1
        if end + 5 > len(query):
            return
        question = query[12:end + 5]
        is_ipv4 = query[end + 1:end + 3] == b"\x00\x01"
        header = query[:2] + struct.pack("!HHHHH", 0x8180, 1, int(is_ipv4), 0, 0)
        answer = b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x00\x00\x04\x7f\x00\x00\x01" if is_ipv4 else b""
        connection.sendto(header + question + answer, self.client_address)


def check_rule_equivalence(core, root, fixture, opener):
    outcomes = {}
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), Echo) as echo, socketserver.ThreadingUDPServer(("127.0.0.1", 0), LocalDNS) as dns:
        echo.daemon_threads = dns.daemon_threads = True
        workers = [threading.Thread(target=server.serve_forever, daemon=True) for server in (echo, dns)]
        for worker in workers:
            worker.start()
        try:
            for variant in ("source", "compiled"):
                proxy_port, control_port = free_port(), free_port()
                text = fixture[variant].replace("__DNS_PORT__", str(dns.server_address[1]))
                text += f"\nmixed-port: {proxy_port}\n"
                config = root / f"{fixture['name']}-{variant}.yaml"
                config.write_text(text, encoding="utf-8")
                command = [str(core), "-d", str(root), "-f", str(config)]
                tested = subprocess.run(command + ["-t"], capture_output=True, timeout=20)
                if tested.returncode:
                    raise RuntimeError(f"Core rejected synthetic rule {variant}")
                process = subprocess.Popen(command + ["-ext-ctl", f"127.0.0.1:{control_port}", "-secret", "synthetic-test-secret"],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                    creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                try:
                    deadline = time.monotonic() + 10
                    while True:
                        try:
                            controller(opener, control_port, "/version")
                            break
                        except (urllib.error.URLError, TimeoutError):
                            if process.poll() is not None or time.monotonic() >= deadline:
                                raise RuntimeError("Synthetic rule controller did not start") from None
                            time.sleep(0.1)
                    chains = []
                    for case in fixture["requests"]:
                        with socket.create_connection(("127.0.0.1", proxy_port), timeout=5) as connection:
                            source_port = str(connection.getsockname()[1])
                            destination = f"{case['host']}:{echo.server_address[1]}"
                            connection.sendall(f"CONNECT {destination} HTTP/1.1\r\nHost: {destination}\r\n\r\n".encode("ascii"))
                            response = b""
                            while b"\r\n\r\n" not in response and len(response) < 4096:
                                response += connection.recv(4096)
                            if not response.startswith(b"HTTP/1.1 200"):
                                raise RuntimeError("Synthetic local CONNECT failed")
                            connection.sendall(b"synthetic-rule-check")
                            if connection.recv(128) != b"synthetic-rule-check":
                                raise RuntimeError("Synthetic local echo failed")
                            active = controller(opener, control_port, "/connections")["connections"]
                            matched = [item for item in active if item["metadata"]["sourcePort"] == source_port]
                            if len(matched) != 1 or matched[0]["chains"] != ["DIRECT", case["policy"]]:
                                raise RuntimeError(f"Unexpected {variant} rule selection for synthetic case {case['host']}")
                            chains.append(matched[0]["chains"])
                    outcomes[variant] = chains
                finally:
                    stop(process)
        finally:
            echo.shutdown()
            dns.shutdown()
            for worker in workers:
                worker.join(timeout=5)
    if outcomes["source"] != outcomes["compiled"]:
        raise RuntimeError("Rule compilation changed effective policy selection")
    return {"case": fixture["name"], "config_test": "passed", "equivalent_requests": len(fixture["requests"])}
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
            if fixture.get("kind") == "rule_equivalence":
                results.append(check_rule_equivalence(core, root, fixture, opener))
                continue
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
