"""Unix PTY smoke check. Run after cargo build: python3 tests/tui_pty.py."""
import fcntl
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


class Session:
    def __init__(self, workspace):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.resize(80, 24)
        self.transcript = b""
        start = time.monotonic()
        self.process = subprocess.Popen(
            [str(BINARY), workspace], stdin=self.slave, stdout=self.slave,
            stderr=self.slave,
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
        session.send(b"fp", b"preview through the real terminal")
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
        session.send(b":unknown", b":unknown_")
        session.send(b"\r", b"Error: Unknown command: unknown")
        session.send(b"\x1b", b"> Items")
        session.send(b"/notes\r", b"filter: notes")
        session.send(b"a", b"Actions |")
        session.send(b"\r", b"| /notes with spaces/ |")
        open_ms = session.last_event_ms
        session.send(b"fp", b"nested terminal preview")
        session.send(b"\x1b", b"> Items")
        session.send(b"\x7f", b"\x1b[7m> notes with spaces/")
        parent_ms = session.last_event_ms
        session.send(b"fo", b"| /notes with spaces/ |")
        session.send(b":files.parent\r", b"\x1b[7m> notes with spaces/")
        session.send(b":files.open notes with spaces/empty\r", b"No entries")
        session.send(b"\x7f", b"\x1b[7m> empty/")
        session.send(b"\x7f", b"\x1b[7m> notes with spaces/")
        session.resize(20, 6)
        session.wait_for(b"Terminal too small")
        session.resize(80, 24)
        session.wait_for(b"> Items")
        session.exit(b"\x03")
        print(f"PTY workflow, nested/empty directories, parent selection, Unicode, arrows, resize and Ctrl-C: passed (first frame {session.startup_ms:.1f} ms; directory {open_ms:.1f} ms; parent {parent_ms:.1f} ms)")
    finally:
        session.close()

    for key, name in [(b"q", "q"), (b"\x04", "Ctrl-D")]:
        session = Session(workspace)
        try:
            session.exit(key)
            print(f"{name} restores terminal: passed")
        finally:
            session.close()
