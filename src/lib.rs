mod error;

pub use {
    error::*,
};

/// Query the xterm interface, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
pub fn query<MS: Into<u64>>(
    query: &str,
    timeout_ms: MS,
) -> Result<String, XQError> {
    // I'll use <const N: usize = 100> as soon as default values for const generics
    // are stabilized. See https://github.com/rust-lang/rust/issues/44580
    const N: usize = 100;
    let mut response = [0; N];
    let n = query_buffer(query, &mut response, timeout_ms.into())?;
    let s = std::str::from_utf8(&response[..n])?;
    Ok(s.to_string())
}

/// Query the xterm interface, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
///
/// Return the number of bytes read.
#[cfg(unix)]
pub fn query_buffer<MS: Into<u64>>(
    query: &str,
    buffer: &mut [u8],
    timeout_ms: MS,
) -> Result<usize, XQError> {
    use mio::{unix::SourceFd, Events, Poll, Interest, Token};
    use std::io::{self, Read, Write};
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "{}", query)?;
    stdout.flush()?;
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(1024);
    let mut stdin_fd = SourceFd(&nix::libc::STDIN_FILENO); // fancy way to pass the 0 const
    poll.registry().register(
        &mut stdin_fd,
        Token(0),
        Interest::READABLE,
    )?;
    let timeout = std::time::Duration::from_millis(timeout_ms.into());
    poll.poll(&mut events, Some(timeout))?;
    for event in &events {
        if event.token() == Token(0) {
            let bytes_written = stdin.read(buffer)?;
            return Ok(bytes_written)
        }
    }
    Err(XQError::Timeout) // no file descriptor was ready in time
}

#[cfg(not(unix))]
pub fn query_buffer(
    _query: &str,
    _buffer: &mut [u8],
    _timeout_ms: u64,
) -> Result<usize, XQError> {
    Err(XQError::Unsupported)
}

