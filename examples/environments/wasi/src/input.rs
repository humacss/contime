//! Nonblocking control bytes from a host-owned pipe; independent of application cases.
use std::io::{self, Read};
#[derive(Clone, Copy)]
pub enum Mode {
    Nonblocking,
    Poll,
}
pub struct Input {
    mode: Mode,
    closed: bool,
}
impl Input {
    pub fn new(mode: Mode) -> io::Result<Self> {
        #[cfg(any(unix, target_os = "wasi"))]
        if matches!(mode, Mode::Nonblocking) {
            // Node supports descriptor flags but its zero-timeout poll omits
            // ready bytes. Configure this private control pipe once.
            let flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if unsafe { libc::fcntl(0, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(not(any(unix, target_os = "wasi")))]
        {
            return Err(io::Error::other("Nonblocking guest input is unavailable on this host"));
        }
        #[allow(unreachable_code)]
        Ok(Self { mode, closed: false })
    }
    pub fn read(&mut self) -> io::Result<Vec<u8>> {
        if self.closed {
            return Ok(Vec::new());
        }
        #[cfg(any(unix, target_os = "wasi"))]
        if matches!(self.mode, Mode::Poll) {
            // Wasmtime 36's legacy threaded host supports readiness polling,
            // but does not grant stdin rights to change descriptor flags.
            let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut fd, 1, 0) };
            if ready < 0 {
                return Err(io::Error::last_os_error());
            }
            if ready == 0 || fd.revents & (libc::POLLIN | libc::POLLHUP) == 0 {
                return Ok(Vec::new());
            }
        }
        let mut bytes = [0u8; 64];
        match io::stdin().read(&mut bytes) {
            Ok(n) => {
                self.closed = n == 0;
                Ok(bytes[..n].to_vec())
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }
}
