mod error;

use {nix::errno::Errno, std::os::fd::BorrowedFd};

pub use error::*;

/// Query the xterm interface, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
pub fn query<MS: Into<u64>>(query: &str, timeout_ms: MS) -> Result<String, XQError> {
    // I'll use <const N: usize = 100> when default values for const generics
    // are stabilized for enough rustc versions
    // See https://github.com/rust-lang/rust/issues/44580
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
    use std::{
        fs::File,
        io::{self, Read, Write},
        os::fd::AsFd,
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "{}", query)?;
    stdout.flush()?;
    let mut stdin = File::open("/dev/tty")?;
    let stdin_fd = stdin.as_fd();

    match wait_for_input(stdin_fd, timeout_ms) {
        Ok(0) => Err(XQError::Timeout),
        Ok(_) => {
            let bytes_written = stdin.read(buffer)?;
            Ok(bytes_written)
        }
        Err(e) => Err(XQError::IO(e.into())),
    }
}

#[cfg(not(unix))]
pub fn query_buffer(_query: &str, _buffer: &mut [u8], _timeout_ms: u64) -> Result<usize, XQError> {
    Err(XQError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
fn wait_for_input<MS: Into<u64>>(fd: BorrowedFd<'_>, timeout_ms: MS) -> Result<i32, Errno> {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

    let poll_fd = PollFd::new(fd, PollFlags::POLLIN);
    let timeout = PollTimeout::try_from(timeout_ms.into()).map_err(|_| Errno::EOVERFLOW)?;

    poll(&mut [poll_fd], timeout)
}

// On MacOS, we need to use the `select` instead of `poll` because it doesn't support poll with tty:
//
// https://github.com/tokio-rs/mio/issues/1377
#[cfg(target_os = "macos")]
fn wait_for_input<MS: Into<u64>>(fd: BorrowedFd<'_>, timeout_ms: MS) -> Result<i32, Errno> {
    use {
        nix::sys::{
            select::{select, FdSet},
            time::TimeVal,
        },
        std::{os::fd::AsRawFd, time::Duration},
    };
    let mut fd_set = FdSet::new();
    fd_set.insert(fd);
    let timeout_us = Duration::from_millis(timeout_ms.into())
        .as_micros()
        .try_into()
        .map_err(|_| Errno::EOVERFLOW)?;
    let mut tv = TimeVal::new(0, timeout_us);

    select(
        fd.as_raw_fd() + 1,
        Some(&mut fd_set),
        None,
        None,
        Some(&mut tv),
    )
}
