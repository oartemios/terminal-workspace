"""Unix PTY smoke check. Run after cargo build: python3 tests/tui_pty.py."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time


BINARY = Path(__file__).resolve().parents[1] / "target/debug/tw"
PLUGIN_STORE = tempfile.TemporaryDirectory(prefix="tw-pty-global-plugins-")


class Session:
    def __init__(self, workspace):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.resize(80, 24)
        self.transcript = b""
        start = time.monotonic()
        self.process = subprocess.Popen(
            [str(BINARY), workspace], stdin=self.slave, stdout=self.slave,
            stderr=self.slave, env={**os.environ, "TW_PLUGIN_DIR": PLUGIN_STORE.name},
        )
        self.wait_for(b"Space: palette")
        self.startup_ms = (time.monotonic() - start) * 1000
        assert not termios.tcgetattr(self.slave)[3] & termios.ICANON

    def resize(self, width, height):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))

    def read(self, timeout):
        if select.select([self.master], [], [], timeout)[0]:
            chunk = os.read(self.master, 65536)
            self.transcript += chunk
            return chunk
        return b""

    def drain(self):
        while self.read(0.02):
            pass

    def wait_for(self, expected):
        data = b""
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            data += self.read(0.05)
            if expected in data:
                return data
            assert self.process.poll() is None, data.decode("utf-8", errors="replace")
        raise AssertionError(f"Missing {expected!r}: {data.decode('utf-8', errors='replace')}")

    def send(self, data, expected):
        self.drain()
        start = time.monotonic()
        os.write(self.master, data)
        response = self.wait_for(expected)
        self.last_event_ms = (time.monotonic() - start) * 1000
        return response

    def exit(self, key):
        self.drain()
        os.write(self.master, key)
        assert self.process.wait(timeout=3) == 0
        self.drain()
        restored = termios.tcgetattr(self.slave)
        expected = self.original.copy()
        # BSD kernels may set PENDIN (pending input state) when canonical mode
        # is restored; compare configuration flags without that transient state.
        restored[3] &= ~termios.PENDIN
        expected[3] &= ~termios.PENDIN
        assert restored == expected, f"termios was not restored: before={expected!r}, after={restored!r}"
        assert b"\x1b[?25h\x1b[?1049l" in self.transcript, "screen/cursor was not restored"

    def close(self):
        if self.process.poll() is None:
            # Try a normal exit so a failed assertion does not leave a raw slave.
            os.write(self.master, b"\x03")
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        os.close(self.master)
        os.close(self.slave)


with tempfile.TemporaryDirectory(prefix="tw-pty-") as workspace:
    Path(workspace, "заметка.md").write_text("preview through the real terminal\n", encoding="utf-8")
    Path(workspace, "notes with spaces/empty").mkdir(parents=True)
    Path(workspace, "notes with spaces/child.txt").write_text("nested terminal preview\n", encoding="utf-8")
    session = Session(workspace)
    try:
        session.send(b"\t", b"> Plugins")
        session.send(b"\r", b"> Groups")
        session.send(b"\r", b"> Items")
        session.send(b"/", b"/_")
        session.send("заметка".encode(), "заметка_".encode())
        session.send(b"\r", "filter: заметка".encode())
        session.send(b"p", b"preview through the real terminal")
        session.send(b"\x1b", b"> Items")
        session.send(b"a", b"Actions |")
        session.send(b"\x1b[B", b"\x1b[7m> Preview file")
        session.send(b"\r", b"preview through the real terminal")
        session.send(b"\x1b", b"> Items")
        session.send(b" ", b"Command palette")
        session.send(b"preview", b"Command palette > preview")
        session.send(b"\r", b"preview through the real terminal")
        session.send(b"\x1b", b"> Items")
        session.send(b":files.path", b":files.path_")
        session.send(b"\r", workspace.encode())
        session.send(b"\x1b", b"> Items")
        session.send(b"a", b"Actions |")
        session.send(b"\r", workspace.encode())
        session.send(b"\x1b", b"> Items")
        session.send(b" ", b"Command palette")
        session.send(b"files.path", b"Command palette > files.path")
        session.send(b"\r", workspace.encode())
        session.send(b"\x1b", b"> Items")
        session.send(b":unknown", b":unknown_")
        session.send(b"\r", b"Error: Unknown command: unknown")
        session.send(b"\x1b", b"> Items")
        session.send(b"/notes\r", b"filter: notes")
        session.send(b"a", b"Actions |")
        session.send(b"\r", b"| /notes with spaces/ |")
        open_ms = session.last_event_ms
        session.send(b"p", b"nested terminal preview")
        session.send(b"\x1b", b"> Items")
        session.send(b"h", "\x1b[7m> ▸ notes with spaces/".encode())
        parent_ms = session.last_event_ms
        session.send(b"l", b"| /notes with spaces/ |")
        session.send(b":files.parent\r", "\x1b[7m> ▸ notes with spaces/".encode())
        session.send(b":files.open notes with spaces/empty\r", b"No entries")
        session.send(b"\x7f", "\x1b[7m> ▸ empty/".encode())
        session.send(b"\x7f", "\x1b[7m> ▸ notes with spaces/".encode())
        session.resize(20, 6)
        session.wait_for(b"Terminal too small")
        session.resize(80, 24)
        session.wait_for(b"> Items")
        session.resize(120, 32)
        session.wait_for("╭".encode())
        session.send(b"\x1b", b"> Items")
        session.send("/заметка\r".encode(), "filter: заметка".encode())
        wide_output = session.send(b"p", b"preview through the real terminal")
        assert "> ▤ заметка.md".encode() in wide_output
        session.send(b"\x1b", b"> Items")
        session.send(b"a", b"Actions |")
        session.send(b"\x1b", b"> Items")
        session.send(b" ", b"Command palette")
        session.send(b"\x1b", b"> Items")
        session.exit(b"\x03")
        print(f"PTY workflow, nested/empty directories, parent selection, Unicode, arrows, resize and Ctrl-C: passed (first frame {session.startup_ms:.1f} ms; directory {open_ms:.1f} ms; parent {parent_ms:.1f} ms)")
    finally:
        session.close()

    # Uninstall and reinstall the same first-party package through the public loader.
    # This also checks that first-run bootstrap does not undo an explicit uninstall.
    with tempfile.TemporaryDirectory(prefix="tw-package-source-") as package_parent:
        package = str(Path(package_parent, "Files package"))
        env = {**os.environ, "TW_PLUGIN_DIR": PLUGIN_STORE.name}
        subprocess.run([str(BINARY), "plugins", "package-files", package], env=env, check=True, capture_output=True)
        subprocess.run([str(BINARY), "plugins", "uninstall", "files"], env=env, check=True, capture_output=True)
        session = Session(workspace)
        try:
            assert b"No entries" in session.transcript
            session.send(b" ", b"Command palette")
            session.send(b"core.plugin.install\r", b":core.plugin.install ")
            session.send(package.encode() + b"\r", b"files: installed")
            session.send(b"\x1b", b"untrusted")
            session.send(b":core.plugins\r", b"Availability: Untrusted")
            session.send(b"\x1b", b"> Items")
            session.send(b":core.plugin.trust files\r", b"trusted for local")
            session.send(b"\x1b", b"> Items")
            session.send("/заметка\rp".encode(), b"preview through the real terminal")
            session.send(b"\x1b", b"> Items")
            session.send(b",s", b"session disabled")
            session.send(b"\x1b", b"> Items")
            session.send(b":core.plugins\r", b"Connection: Disconnected")
            session.send(b"\x1b", b"> Items")
            session.send(b",e", b"enabled for Workspace and session")
            session.send(b"\x1b", b"> Items")
            session.exit(b"q")
            print("Public package CLI, uninstall/bootstrap separation, palette install, trust, saved permissions and process lifecycle: passed")
        finally:
            session.close()

    with tempfile.TemporaryDirectory(prefix="tw other project ") as second:
        Path(second, "second.txt").write_text("second project preview", encoding="utf-8")
        Path(second, ".terminal-workspace.json").write_text(json.dumps({
            "version": 1, "plugins": {"files": {"enabled": True, "permissions": ["WorkspaceRead"]}},
            "overrides": {"keybindings": [
                {"keys": "p", "plugin": "files", "command": None},
                {"keys": "xyz", "plugin": "files", "command": "files.preview"},
            ]},
        }), encoding="utf-8")
        session = Session(workspace)
        try:
            session.send("/заметка\rp".encode(), b"preview through the real terminal")
            session.send(b"\x1b", b"> Items")
            session.send(b",w", b":core.workspace.open ")
            session.send(second.encode() + b"\r", b"second.txt")
            session.send(b"/second\r", b"filter: second")
            session.send(b"xy", b"xy ")
            session.send(b"\x1b", b"> Items")
            session.send(b"xyz", b"second project preview")
            session.send(b"\x1b", b"> Items")
            session.send(b",b", "filter: заметка".encode())
            session.send(b"p", b"preview through the real terminal")
            session.send(b"\x1b", b"> Items")
            session.send(b":core.workspace.open missing-project\r", b"Error:")
            session.send(b"p", b"preview through the real terminal")
            session.exit(b"q")
            print("Workspace switch, path with spaces, local bindings, overrides, prefix cancellation, context restoration and failed-open recovery: passed")
        finally:
            session.close()

    for key, name in [(b"q", "q"), (b"\x04", "Ctrl-D")]:
        session = Session(workspace)
        try:
            session.exit(key)
            print(f"{name} restores terminal: passed")
        finally:
            session.close()

    for command, expected in [
        (b",s", b"files: session disabled"),
        (b":core.plugins\r", b"files: active"),
        (b",d", b"files: workspace disabled"),
        (b":core.plugins\r", b"files: workspace disabled"),
        (b",e", b"enabled for Workspace and session"),
        (b",r", b"WorkspaceRead revoked"),
    ]:
        session = Session(workspace)
        try:
            session.send(command, expected)
            session.exit(b"q")
        finally:
            session.close()
    session = Session(workspace)
    try:
        assert b"WorkspaceRead not granted" in session.transcript
        session.send(b" ", b"Command palette")
        session.send(b"core.permission.grant", b"core.permission.grant")
        session.send(b"\r", b":core.permission.grant files ")
        session.send(b"WorkspaceRead\r", b"WorkspaceRead granted")
        session.send(b"\x1b", b"> Items")
        session.send("/заметка\rp".encode(), b"preview through the real terminal")
        session.exit(b"q")
        print("Activation and permission commands, restart persistence, palette recovery: passed")
    finally:
        session.close()
