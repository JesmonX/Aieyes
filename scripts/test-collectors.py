import contextlib
import io
import json
import pathlib
import runpy
import tempfile
import types
import unittest
from unittest.mock import patch

SCRIPTS = pathlib.Path(__file__).resolve().parent

class Collectors(unittest.TestCase):
    def test_remote_history_filters_conversation_text(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "session.jsonl"
            events = [
                {"type": "session_meta", "payload": {"id": "s", "base_instructions": "private-text"}},
                {"type": "turn_context", "payload": {"model": "test", "cwd": "private-path"}},
                {"type": "response_item", "payload": {"text": "private-text"}},
                {"type": "event_msg", "timestamp": "2026-10-03T00:00:00Z", "payload": {"type": "token_count", "info": {"total_token_usage": {"input_tokens": 10}}, "rate_limits": None}},
            ]
            path.write_text("\n".join(json.dumps(e) for e in events) + "\n")
            output = io.StringIO()
            with patch("sys.argv", ["collector", directory, "codex"]), contextlib.redirect_stdout(output):
                runpy.run_path(str(SCRIPTS / "remote_history.py"))
            text = output.getvalue()
            self.assertNotIn("private-text", text)
            self.assertNotIn("private-path", text)
            self.assertEqual(len(json.loads(text)["files"][0]["events"]), 3)

    def test_remote_quota_returns_only_usage_and_sanitizes_errors(self):
        module = runpy.run_path(str(SCRIPTS / "remote_quota.py"))
        with tempfile.TemporaryDirectory() as directory:
            (pathlib.Path(directory) / ".credentials.json").write_text(json.dumps({"claudeAiOauth": {"accessToken": "fixture-token"}}))
            with patch("sys.argv", ["quota", directory]), patch("urllib.request.urlopen", return_value=io.StringIO(json.dumps({"five_hour": {"utilization": 20}, "extra": "private"}))):
                self.assertEqual(module["main"](), {"five_hour": {"utilization": 20}})
            with patch("sys.argv", ["quota", directory]), patch("urllib.request.urlopen", side_effect=OSError("fixture-token")):
                response = module["main"]()
                self.assertIn("error", response)
                self.assertNotIn("fixture-token", json.dumps(response))

    def test_linux_units_guest_cpu_and_multiple_gpus(self):
        files = {
            "/proc/sys/kernel/random/boot_id": "boot-a\n", "/proc/uptime": "120.5 30\n", "/proc/loadavg": "1.2 2.3 3.4 1/20 9\n",
            "/proc/stat": "cpu 10 2 8 60 5 3 2 10 40 20\ncpu0 10 2 8 60 5 3 2 10 40 20\n",
            "/proc/meminfo": "MemTotal: 1024 kB\nMemAvailable: 768 kB\nCached: 128 kB\nBuffers: 32 kB\nSwapTotal: 64 kB\nSwapFree: 32 kB\n",
            "/proc/diskstats": "8 0 sda 10 0 20 0 30 0 40 0 0 50 0\n",
            "/proc/net/dev": "header\nheader\neth0: 1024 10 1 2 0 0 0 0 2048 20 3 4 0 0 0 0\n",
        }
        output = io.StringIO()
        original_open = open
        def fake_open(path, *args, **kwargs):
            return io.StringIO(files[str(path)]) if str(path) in files else original_open(path, *args, **kwargs)
        gpu = types.SimpleNamespace(stdout="0, Test GPU A, 20, 100, 1000, 50, 70.5\n1, Test GPU B, 30, 200, 2000, N/A, N/A\n")
        with patch("builtins.open", side_effect=fake_open), patch("subprocess.run", return_value=gpu), patch("sys.argv", ["collector", '["cpu","memory","disk","network","gpu"]']), contextlib.redirect_stdout(output):
            runpy.run_path(str(SCRIPTS / "linux_metrics.py"))
        data = json.loads(output.getvalue())
        self.assertEqual(data["cpu"][0]["total"], 100)
        self.assertEqual(data["memory"]["total"], 1024 * 1024)
        self.assertEqual(data["disk"][0]["readBytes"], 20 * 512)
        self.assertEqual(data["network"][0]["txDrops"], 4)
        self.assertEqual(len(data["gpu"]), 2)
        self.assertIsNone(data["gpu"][1]["temperature"])
        self.assertNotIn("filesystems", data)

    def test_mount_selection_data_and_warm_sample(self):
        files = {
            "/proc/sys/kernel/random/boot_id": "boot-a", "/proc/uptime": "123 0", "/proc/loadavg": "0 0 0",
            "/proc/mounts": "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /data\\040space xfs rw 0 0\nproc /proc proc rw 0 0\n",
        }
        original_open = open
        def fake_open(path, *args, **kwargs):
            return io.StringIO(files[str(path)]) if str(path) in files else original_open(path, *args, **kwargs)
        stats = types.SimpleNamespace(f_blocks=1000, f_frsize=4096, f_bavail=300, f_bfree=400, f_files=200, f_ffree=120)
        output = io.StringIO()
        # The remote Linux collector is tested on Windows too, where statvfs is absent.
        with patch("builtins.open", side_effect=fake_open), patch("os.statvfs", return_value=stats, create=True), patch("time.sleep"), patch("time.monotonic", side_effect=[10.0, 10.25]), patch("sys.argv", ["collector", '["filesystems"]', "true"]), contextlib.redirect_stdout(output):
            runpy.run_path(str(SCRIPTS / "linux_metrics.py"))
        sample = json.loads(output.getvalue())
        self.assertEqual([f["id"] for f in sample["filesystems"]], ["/", "/data space"])
        self.assertEqual(sample["filesystems"][0]["used"], 600 * 4096)
        self.assertEqual(sample["filesystems"][0]["available"], 300 * 4096)
        self.assertEqual(sample["filesystems"][0]["inodesFree"], 120)
        self.assertEqual(sample["monotonic"] - sample["previous"]["monotonic"], 0.25)

if __name__ == "__main__":
    unittest.main()
