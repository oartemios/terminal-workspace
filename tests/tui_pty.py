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


BINARY = Path(os.environ["TW_TEST_BINARY"]).resolve() if "TW_TEST_BINARY" in os.environ else Path(__file__).resolve().parents[1] / "target/debug/tw"
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
        try:
            self.wait_for(b"Space: palette")
        except Exception:
            self.close()
            raise
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
        deadline = time.monotonic() + 10
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
    Path(workspace, "long-preview.md").write_text("".join(f"preview line {i}\n" for i in range(1, 2001)))
    Path(workspace, "rendered.md").write_text("# Heading Example\n- **bold item**\n```rust\nlet x = \"**literal**\";\n```\n> Quote\n")
    Path(workspace, "wrapped-scroll.md").write_text("".join(f"source line {i}\n" for i in range(1, 19)) + "word " * 600 + "\nwrapped scroll tail\n")
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
        session.send(b":files.preview long-preview.md\r", b"preview line 1")
        session.send(b"\x1b[F", b"preview line 2000")
        command_frame = session.send(b":files.path", b":files.path_")
        assert b"preview line 2000" in command_frame
        session.send(b"\x1b", b"preview line 2000")
        error_frame = session.send(b":unknown\r", b"Error: Unknown command: unknown")
        assert b"preview line 2000" in error_frame
        session.send(b":files.path long-preview.md\r", b"long-preview.md")
        session.send(b":files.preview long-preview.md\r", b"preview line 1")
        session.send(b"\x1b[H", b"preview line 1")
        session.send(b"/preview line 1800", b"Find /preview line 1800_")
        session.send(b"\r", b"match 1/1")
        search_ms = session.last_event_ms
        session.send(b"n", b"preview line 1800")
        session.send(b"g120", b"Line > 120_")
        session.send(b"\r", b"line 120/2000")
        goto_ms = session.last_event_ms
        session.send(b":1800\r", b"line 1800/2000")
        session.send(b":files.preview rendered.md\r", b"Heading Example")
        raw = session.send(b"v", b"- **bold item**")
        assert b"# Heading Example" in raw
        assert b"- **bold item**" in raw
        rendered = session.send(b"v", "• bold item".encode())
        assert b"# Heading Example" not in rendered
        session.send(b"/**literal**\r", b"match 1/1")
        session.send(b":files.preview wrapped-scroll.md\r", b"source line 1")
        session.send(b"j" * 120, b"wrapped scroll tail")
        session.send(b"k" * 120, b"source line 1")
        session.send(b"\x1b", b"> Items")
        session.send(b"a", b"Actions |")
        session.send(b"\x1b", b"> Items")
        session.send(b" ", b"Command palette")
        session.send(b"\x1b", b"> Items")
        session.exit(b"\x03")
        print(f"PTY workflow, Unicode, resize, output commands/search/source-line jump, Markdown/source and Ctrl-C: passed (first frame {session.startup_ms:.1f} ms; directory {open_ms:.1f} ms; parent {parent_ms:.1f} ms; search {search_ms:.1f} ms; goto {goto_ms:.1f} ms)")
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
            session.send(b"\x1b", "заметка.md".encode())
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

# Git is explicitly installed and activated; it uses the same loader as Files.
with tempfile.TemporaryDirectory(prefix="tw-git-pty-") as base:
    env = {**os.environ, "TW_PLUGIN_DIR": PLUGIN_STORE.name}
    roots = [Path(base, name) for name in ("first", "second")]
    for index, root in enumerate(roots):
        root.mkdir()
        def git(*args):
            subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)
        git("init", "-b", "main")
        git("config", "user.name", "PTY Fixture")
        git("config", "user.email", "pty@example.test")
        (root / "note with spaces.txt").write_text("before\n")
        git("add", ".")
        git("commit", "-m", "initial")
        (root / "note with spaces.txt").write_text(f"after project {index}\n")
    package = str(Path(base, "Git package"))
    subprocess.run([str(BINARY), "plugins", "package-git", package], env=env, check=True, capture_output=True)
    subprocess.run([str(BINARY), "plugins", "install", package], env=env, check=True, capture_output=True)
    subprocess.run([str(BINARY), "plugins", "trust", "git"], env=env, check=True, capture_output=True)
    roots[1].joinpath(".terminal-workspace.json").write_text(json.dumps({
        "version": 1, "plugins": {"git": {"enabled": True, "permissions": ["WorkspaceRead", "Process"]}},
    }))
    session = Session(str(roots[0]))
    try:
        session.resize(140, 32)
        session.wait_for("╭".encode())
        session.send(b":core.plugin.enable git\r", b"enabled for Workspace and session")
        session.send(b"\x1b", b"> Items")
        for permission in [b"WorkspaceRead", b"Process"]:
            session.send(b":core.permission.grant git " + permission + b"\r", permission + b" granted")
            session.send(b"\x1b", b"> Items")
        session.send(b"\t\x1b[C\r\r", b"note with spaces.txt")
        session.send(b"/note with spaces\r", b"filter: note with spaces")
        session.send(b"a", b"View file diff")
        session.send(b"\r", b"+after project 0")
        session.send(b"\x1b", b"> Items")
        session.send(b"p", b"+after project 0")
        session.send(b"\x1b", b"> Items")
        session.send(b"\x1b[Z\x1b[C\r", b"* main")
        session.send(b"p", b"Branch: main")
        session.send(b"\x1b", b"> Items")
        session.send(b",w", b":core.workspace.open ")
        session.send(str(roots[1]).encode() + b"\r", b"note with spaces.txt")
        session.send(b"\t\x1b[C\r\r", b"note with spaces.txt")
        session.send(b":git.diff note with spaces.txt\r", b"+after project 1")
        session.send(b"\x1b", b"> Items")
        session.send(b",b", b"* main")
        session.send(b"p", b"Branch: main")
        session.send(b"\x1b", b"> Items")
        session.send(b":core.plugin.suspend git\r", b"session disabled")
        session.send(b"\x1b", b"> Items")
        session.send(b"\t\x1b[D\r\r", b"[Entries]")
        session.send(b":files.preview note with spaces.txt\r", b"after project 0")
        session.exit(b"q")
        print("Git package CLI, explicit activation/permissions, Status/diff, Actions, scoped binding, Branches, two Workspaces and Files after suspend: passed")
    finally:
        session.close()


# Protocol gates make the actual terminal test independent of network speed.
with tempfile.TemporaryDirectory(prefix="tw-background-pty-") as base:
    root = Path(base, "project")
    root.mkdir()
    (root / "note.txt").write_text("local preview while worker is blocked\n")
    source = Path(base, "Probe package")
    source.mkdir()
    source.joinpath("plugin").write_bytes(Path(__file__).parent.joinpath("fixtures/runtime_plugin.py").read_bytes())
    source.joinpath("plugin.json").write_text(json.dumps({
        "package_version": 1, "protocol_version": 1, "api_version": "0.6",
        "executable": "plugin", "args": ["gate_describe", "gate_view"],
        "environment": [], "credentials": [],
        "plugin": {"id": "probe", "name": "Probe", "groups": [{"id": "entries", "title": "Entries"}],
                   "commands": [{"id": "probe.info", "title": "Probe info", "requires_item": False}],
                   "permissions": [], "keybindings": []},
    }))
    installed = Path(PLUGIN_STORE.name, "probe")
    def marker(operation):
        deadline = time.monotonic() + 5
        path = installed / f"{operation}.started"
        while time.monotonic() < deadline:
            if path.exists() and path.read_text().strip():
                return int(path.read_text())
            time.sleep(0.005)
        raise AssertionError(f"Missing gate {operation}")
    session = Session(str(root))
    try:
        session.send(b"r", b"note.txt")
        for command, expected in [
            (f"core.plugin.install {source}", b"probe: installed"),
            ("core.plugin.trust probe", b"trusted for local"),
            ("core.plugin.enable probe", b"enabled for Workspace and session"),
        ]:
            session.send(b":" + command.encode() + b"\r", expected)
            session.send(b"\x1b", b"note.txt")
        session.send(b":probe.info\r", b"Loading: probe")
        marker("describe")
        session.send(b"?", b"Tab / Shift-Tab")
        assert session.last_event_ms < 500, session.last_event_ms
        session.send(b"\x1b", b"note.txt")
        session.send(b":files.preview note.txt\r", b"local preview while worker is blocked")
        assert session.last_event_ms < 500, session.last_event_ms
        (installed / "describe.release").write_text("go")
        session.send(b"\x1b", b"> Items")
        session.send(b"\t\x1b[C\x1b[C\r\r", b"Loading: probe")
        marker("view")
        session.send(b"\t\x1b[D\x1b[D\r\r", b"note.txt")
        assert session.last_event_ms < 500, session.last_event_ms
        (installed / "view.release").write_text("go")
        session.send(b"?", b"Tab / Shift-Tab")
        session.send(b"\x1b", b"note.txt")
        assert b"Probe item" not in session.read(0.05)
        session.send(b":core.plugin.suspend probe\r", b"session disabled")
        session.exit(b"q")
        print("Background startup/view gates, keyboard help/navigation, local Files preview, stale-result suppression and terminal restoration: passed")
    finally:
        session.close()
