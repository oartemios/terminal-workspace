use console::Key;
use std::io::{self, IsTerminal, Write};
use std::mem::MaybeUninit;

/// Owns terminal changes so normal exit, errors and unwinding restore the shell.
pub struct Terminal {
    original: libc::termios,
}

impl Terminal {
    pub fn enter() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Run tw in an interactive terminal",
            ));
        }
        let mut original = MaybeUninit::uninit();
        // SAFETY: stdin is a terminal and tcgetattr initializes this termios on success.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        // SAFETY: raw is initialized; stdin remains owned by this process.
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let terminal = Self { original };
        io::stdout().write_all(b"\x1b[?1049h\x1b[?25l")?;
        io::stdout().flush()?;
        Ok(terminal)
    }

    pub fn size(&self) -> (usize, usize) {
        let mut size = MaybeUninit::<libc::winsize>::zeroed();
        // SAFETY: ioctl writes a winsize into a valid, aligned allocation.
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, size.as_mut_ptr()) } == 0 {
            let size = unsafe { size.assume_init() };
            if size.ws_col > 0 && size.ws_row > 0 {
                return (size.ws_col as usize, size.ws_row as usize);
            }
        }
        (80, 24)
    }

    pub fn read_key(&self) -> io::Result<Option<Key>> {
        let Some(byte) = read_byte(16)? else {
            return Ok(None);
        };
        Ok(Some(match byte {
            b'\x1b' => match read_byte(30)? {
                None => Key::Escape,
                Some(b'[') | Some(b'O') => match read_byte(30)? {
                    Some(b'A') => Key::ArrowUp,
                    Some(b'B') => Key::ArrowDown,
                    Some(b'C') => Key::ArrowRight,
                    Some(b'D') => Key::ArrowLeft,
                    Some(b'H') => Key::Home,
                    Some(b'F') => Key::End,
                    Some(b'Z') => Key::BackTab,
                    Some(first @ b'0'..=b'9') => {
                        let mut sequence = vec![first];
                        while sequence.len() < 16 {
                            let Some(next) = read_byte(30)? else {
                                break;
                            };
                            sequence.push(next);
                            if next == b'~' || next.is_ascii_alphabetic() {
                                break;
                            }
                        }
                        match sequence.as_slice() {
                            b"1~" | b"7~" => Key::Home,
                            b"4~" | b"8~" => Key::End,
                            b"5~" => Key::PageUp,
                            b"6~" => Key::PageDown,
                            b"3~" => Key::Del,
                            _ => Key::Unknown,
                        }
                    }
                    _ => Key::Unknown,
                },
                Some(_) => Key::Unknown,
            },
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Tab,
            127 | 8 => Key::Backspace,
            byte if byte.is_ascii() => Key::Char(byte as char),
            first => {
                let count = match first {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => 1,
                };
                let mut bytes = vec![first];
                for _ in 1..count {
                    let Some(next) = read_byte(100)? else {
                        break;
                    };
                    bytes.push(next);
                }
                std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|text| text.chars().next())
                    .map(Key::Char)
                    .unwrap_or(Key::Unknown)
            }
        }))
    }

    pub fn draw(&self, screen: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(screen.as_bytes())?;
        stdout.flush()
    }
}

fn read_byte(timeout: i32) -> io::Result<Option<u8>> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: fd points to one initialized pollfd for the duration of the call.
    let result = unsafe { libc::poll(&mut fd, 1, timeout) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if result == 0 {
        return Ok(None);
    }
    if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Terminal disconnected",
        ));
    }
    let mut byte = [0];
    // SAFETY: the pointer references a writable byte. Read directly from the fd:
    // Stdin buffering can otherwise hide remaining bytes from the next poll.
    let count = unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) };
    match count {
        1 => Ok(Some(byte[0])),
        0 => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Terminal disconnected",
        )),
        _ => Err(io::Error::last_os_error()),
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: original came from tcgetattr and remains valid for this terminal.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
        let _ = io::stdout().write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}
