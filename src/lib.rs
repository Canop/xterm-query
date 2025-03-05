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
/// Query the xterm interface for an OSC sequence, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
///
/// The query should be a proper OSC sequence (ie already wrapped, eg "\x1b]11;?\x07")
/// as you want it to be sent to stdout but the answer is only the part after the C0 (ESC)
/// and before the OSC terminator (BEL or ESC).
pub fn query_osc<MS: Into<u64>>(query: &str, timeout_ms: MS) -> Result<String, XQError> {
    // I'll use <const N: usize = 100> when default values for const generics
    // are stabilized for enough rustc versions
    // See https://github.com/rust-lang/rust/issues/44580
    const N: usize = 100;
    let mut response = [0; N];
    let resp = query_osc_buffer(query, &mut response, timeout_ms.into())?;
    let s = std::str::from_utf8(resp)?;
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

/// Query the xterm interface for an OSC response, assuming the terminal is in raw mode
/// (or we would block waiting for a newline).
///
/// The provided query should be a proper OSC sequence (ie already wrapped, eg "\x1b]11;?\x07")
///
/// Return a slice of the buffer containing the response. This slice excludes
/// - the response start (ESC) and everything before
/// - the response end (ESC or BEL) and everything after
///
/// OSC sequence:
///  <https://en.wikipedia.org/wiki/ANSI_escape_code#OSC_(Operating_System_Command)_sequences>
#[cfg(unix)]
pub fn query_osc_buffer<'b, MS: Into<u64> + Copy>(
    query: &str,
    buffer: &'b mut [u8],
    timeout_ms: MS,
) -> Result<&'b [u8], XQError> {
    use std::{
        fs::File,
        io::{self, Read, Write},
        os::fd::AsFd,
    };
    const ESC: char = '\x1b';
    const BEL: char = '\x07';

    // Do some casing based on the terminal
    let term = std::env::var("TERM").map_err(|_| XQError::Unsupported)?;
    if term == "dumb" {
        return Err(XQError::Unsupported);
    }
    let is_screen = term.starts_with("screen");

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    // Running under GNU Screen, the commands need to be "escaped",
    // apparently.  We wrap them in a "Device Control String", which
    // will make Screen forward the contents uninterpreted.
    if is_screen {
        write!(stdout, "{ESC}P")?;
    }

    write!(stdout, "{}", query)?;
    // Ask for a Status Report as a "fence". Almost all terminals will
    // support that command, even if they don't support returning the
    // background color, so we can detect "not supported" by the
    // Status Report being answered first.
    write!(stdout, "{ESC}[5n")?;

    if is_screen {
        write!(stdout, "{ESC}\\")?;
    }

    stdout.flush()?;
    let mut stdin = File::open("/dev/tty")?;
    let mut osc_start_idx = None;
    let mut osc_end_idx = None;
    let mut bytes_written = 0;
    while bytes_written < buffer.len() {
        let stdin_fd = stdin.as_fd();
        match wait_for_input(stdin_fd, timeout_ms) {
            Ok(0) => {
                return Err(XQError::Timeout);
            }
            Ok(_) => {
                let bytes_read = stdin.read(&mut buffer[bytes_written..])?;
                if bytes_read == 0 {
                    return Err(XQError::NotAnOSCResponse); // EOF
                }
                // the sequence must start with a ESC (27) and end either with a ESC or BEL (7)
                // then, we'll get an 'n' back from the "fence"
                for i in bytes_written..bytes_written + bytes_read {
                    let b = buffer[i];
                    match osc_start_idx {
                        None => {
                            if b == ESC as u8 {
                                osc_start_idx = Some(i);
                            }
                        }
                        Some(start_idx) => {
                            if b == ESC as u8 || b == BEL as u8 {
                                if osc_end_idx.is_none() {
                                    osc_end_idx = Some(i);
                                }
                            } else if b == b'n' {
                                match osc_end_idx {
                                    None => return Err(XQError::NotAnOSCResponse),
                                    Some(end_idx) => {
                                        return Ok(&buffer[start_idx + 1..=end_idx]);
                                    }
                                }
                            }
                        }
                    }
                }
                bytes_written += bytes_read;
            }
            Err(e) => {
                return Err(XQError::IO(e.into()));
            }
        }
    }
    Err(XQError::BufferOverflow)
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
