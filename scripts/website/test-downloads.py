#!/usr/bin/env python3
"""Exercise the real Caddy routes against a local GitHub latest-release fixture."""
import http.server
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CADDY = os.environ.get("CADDY_BIN") or shutil.which("caddy") or str(ROOT / ".local/website/downloads/caddy")
PACKAGES = {
    "macos/arm64/dmg": "macos-arm64.dmg", "macos/x64/dmg": "macos-x64.dmg",
    "windows/x64/exe": "windows-x64.exe", "linux/x64/appimage": "linux-x64.AppImage",
    "linux/x64/deb": "linux-x64.deb", "linux/x64/rpm": "linux-x64.rpm",
}
state = dict(version="1.2.3", status=302, location=None)


class Latest(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        assert self.path == "/JesmonX/Aieyes/releases/latest", self.path
        assert not self.headers.get("Cookie") and not self.headers.get("Authorization")
        self.send_response(state["status"])
        self.send_header("Location", state["location"] or "https://github.com/JesmonX/Aieyes/releases/tag/v" + state["version"])
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_): pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_): return None


def get(origin, path):
    try:
        response = urllib.request.build_opener(NoRedirect).open(origin + path, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, dict(response.headers), response.read().decode()


def main():
    subprocess.run(["node", str(ROOT / "scripts/website/build.mjs")], check=True, stdout=subprocess.DEVNULL)
    upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Latest)
    threading.Thread(target=upstream.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="aieyes-downloads-") as directory:
        work = Path(directory)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0)); port = sock.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        config = (ROOT / "scripts/website/Caddyfile").read_text()
        config = "{\n admin off\n auto_https off\n}\n" + config.replace("aieyes.2h3ngy4n.space {", origin + " {")
        config = config.replace("/var/www/aieyes/current", str(ROOT / "dist/website"))
        config = config.replace("/var/log/caddy/aieyes-access.log", str(work / "access.log"))
        config = config.replace("reverse_proxy https://github.com", f"reverse_proxy http://127.0.0.1:{upstream.server_port}")
        file = work / "Caddyfile"; file.write_text(config)
        subprocess.run([CADDY, "validate", "--config", str(file), "--adapter", "caddyfile"], check=True, capture_output=True)
        with (work / "process.log").open("w") as log:
            process = subprocess.Popen([CADDY, "run", "--config", str(file), "--adapter", "caddyfile"], stdout=log, stderr=log)
            results = []
            try:
                for _ in range(100):
                    try:
                        if get(origin, "/")[0] == 200: break
                    except OSError: pass
                    if process.poll() is not None: raise AssertionError((work / "process.log").read_text())
                    time.sleep(.05)
                else: raise AssertionError("Caddy did not start")
                for version in ["1.2.3", "99.4.5"]:
                    state.update(version=version)
                    for route, suffix in PACKAGES.items():
                        status, headers, _ = get(origin, "/go/download/" + route)
                        target = f"https://github.com/JesmonX/Aieyes/releases/download/v{version}/Aieyes-{version}-{suffix}"
                        assert status == 302 and headers.get("Location") == target, (route, status, headers)
                        assert "no-store" in headers["Cache-Control"]
                        results.append(dict(route=route, version=version, location=target))
                for status, location in [(500, None), (200, None), (302, "https://untrusted.invalid/releases/tag/v1.2.3"), (302, "https://github.com/JesmonX/Aieyes/releases/tag/unexpected")]:
                    state.update(status=status, location=location)
                    code, headers, body = get(origin, "/go/download/linux/x64/rpm")
                    assert code == 502 and "no-store" in headers["Cache-Control"], (code, headers)
                    assert 'href=""' in body and "Retry" in body and "/releases/latest" in body
                state.update(status=302, location=None)
                assert get(origin, "/go/download/linux/x64/rpm")[0] == 302
                for route, location in {"macos":"/#download-macos", "windows":"/go/download/windows/x64/exe", "linux":"/#download-linux", "latest":"https://github.com/JesmonX/Aieyes/releases/latest"}.items():
                    code, headers, _ = get(origin, "/go/download/" + route)
                    assert code == 302 and headers["Location"] == location
                assert get(origin, "/go/github")[1]["Location"] == "https://github.com/JesmonX/Aieyes"
            finally:
                process.terminate(); process.wait(timeout=10); upstream.shutdown()
            stats = json.loads(subprocess.check_output(["node", str(ROOT / "scripts/website/stats.mjs"), "--file", str(work / "access.log"), "--json"]))
            indexed = {row["path"]:row for row in stats["rows"]}
            for route in PACKAGES:
                row = indexed["/go/download/" + route]
                assert row["clicks"] >= 2 and row["label"].startswith("Download · "), row
            report = ROOT / ".local/website/download-test.json"
            report.write_text(json.dumps(dict(downloads=results, upstreamFailures=4, legacyRoutes=5, statistics=stats), indent=2) + "\n")
            print("Caddy downloads passed: six packages, two latest versions, four upstream failures, retry, legacy routes and package statistics.")


if __name__ == "__main__": main()
