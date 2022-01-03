mod error;

pub use {
    error::*,
};

/// Query the xterm interface, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
pub fn query(
    query: &str,
    timeout_ms: isize,
) -> Result<String, XQError> {
    // I'll use <const N: usize = 100> as soon as default values for const generics
    // are stabilized. See https://github.com/rust-lang/rust/issues/44580
    const N: usize = 100;
    let mut response = [0; N];
    let n = query_buffer(query, &mut response, timeout_ms)?;
    let s = std::str::from_utf8(&response[..n])?;
    Ok(s.to_string())
}

#[cfg(not(unix))]
pub fn query_buffer(
    _query: &str,
    _buffer: &mut [u8],
    _timeout_ms: isize,
) -> Result<usize, XQError> {
    Err(XQError::Unsupported)
}

/// Query the xterm interface, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
///
/// Return the number of bytes read.
#[cfg(unix)]
pub fn query_buffer(
    query: &str,
    buffer: &mut [u8],
    timeout_ms: isize,
) -> Result<usize, XQError> {
    use nix::sys::epoll::*;
    use std::io::{self, Read, Write};
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "{}", query)?;
    stdout.flush()?;
    let poll_fd = epoll_create1(EpollCreateFlags::empty())?;
    let mut event = EpollEvent::new(EpollFlags::EPOLLIN, 0);
    epoll_ctl(
        poll_fd,
        EpollOp::EpollCtlAdd,
        nix::libc::STDIN_FILENO,
        Some(& mut event),
    )?;
    let mut events = [EpollEvent::empty(); 1];
    let fd_count = epoll_wait(poll_fd, &mut events, timeout_ms)?;
    if fd_count == 0 {
        Err(XQError::Timeout) // no file descriptor was ready in time
    } else {
        let bytes_written = stdin.read(buffer)?;
        Ok(bytes_written)
    }
}

