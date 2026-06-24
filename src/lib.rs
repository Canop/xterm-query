//! Query the terminal by writing an escape sequence and reading the reply.
//!
//! The terminal must be in raw mode (otherwise reads block on a newline), and
//! the query should be issued when nothing else is reading terminal input.
//!
//! # Platform support
//!
//! Unix reads the reply from `/dev/tty` using `poll`/`select`. Windows reads it
//! from the console input (`CONIN$`) after switching it to
//! `ENABLE_VIRTUAL_TERMINAL_INPUT` for the duration of the query.
//!
//! ## Windows limitation
//!
//! The Windows wait wakes on any console input event, including focus, mouse,
//! and buffer-resize events, which under VT-input mode may be delivered as
//! escape sequences. If such an event arrives in the query window, the reply
//! can be wrong or empty, so callers should treat an unparsable reply as
//! "unsupported" and degrade gracefully. Issue queries while the terminal is
//! otherwise quiet (e.g. at startup, before an input loop begins).

mod error;

pub use error::*;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{query_buffer, query_osc_buffer};

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

/// Outcome of scanning a (partial) buffer for an OSC response framed as
/// `ESC ... (ESC|BEL)`, followed by the `n` of the DSR ("fence") reply.
///
/// The fence (`ESC[5n` -> `ESC[0n`) lets us detect terminals that don't
/// support the query: they answer the Status Report first, so we meet the
/// `n` before ever seeing a complete OSC.
pub(crate) enum OscScan {
    /// Not enough bytes yet; read more.
    NeedMore,
    /// A complete OSC response was found. `start`/`end` are byte indices into
    /// the scanned buffer; the payload is `buf[start..=end]` (the leading ESC
    /// is excluded, the terminating ESC/BEL is included, matching the
    /// historical Unix behavior).
    Found { start: usize, end: usize },
    /// The fence answered before a complete OSC: not an OSC response.
    NotOsc,
}

/// Scan a buffer for an OSC response. Pure and platform-independent so it can
/// be shared by the Unix and Windows backends and unit-tested anywhere.
pub(crate) fn scan_osc(buf: &[u8]) -> OscScan {
    const ESC: u8 = 0x1b;
    const BEL: u8 = 0x07;
    let mut osc_start: Option<usize> = None;
    let mut osc_end: Option<usize> = None;
    for (i, &b) in buf.iter().enumerate() {
        match osc_start {
            None => {
                if b == ESC {
                    osc_start = Some(i);
                }
            }
            Some(start) => {
                if osc_end.is_none() && (b == ESC || b == BEL) {
                    osc_end = Some(i);
                } else if i >= 3
                    && buf[i - 3] == ESC
                    && buf[i - 2] == b'['
                    && buf[i - 1] == b'0'
                    && b == b'n'
                {
                    return match osc_end {
                        None => OscScan::NotOsc,
                        Some(end) => OscScan::Found {
                            start: start + 1,
                            end,
                        },
                    };
                }
            }
        }
    }
    OscScan::NeedMore
}

#[cfg(unix)]
use {nix::errno::Errno, std::os::fd::BorrowedFd};

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
    let mut filled = 0;
    while filled < buffer.len() {
        let stdin_fd = stdin.as_fd();
        match wait_for_input(stdin_fd, timeout_ms) {
            Ok(0) => {
                return Err(XQError::Timeout);
            }
            Ok(_) => {
                let bytes_read = stdin.read(&mut buffer[filled..])?;
                if bytes_read == 0 {
                    return Err(XQError::NotAnOSCResponse); // EOF
                }
                filled += bytes_read;
                match scan_osc(&buffer[..filled]) {
                    OscScan::Found { start, end } => return Ok(&buffer[start..=end]),
                    OscScan::NotOsc => return Err(XQError::NotAnOSCResponse),
                    OscScan::NeedMore => {}
                }
            }
            Err(e) => {
                return Err(XQError::IO(e.into()));
            }
        }
    }
    Err(XQError::BufferOverflow)
}

#[cfg(not(any(unix, windows)))]
pub fn query_buffer<MS: Into<u64>>(
    _query: &str,
    _buffer: &mut [u8],
    _timeout_ms: MS,
) -> Result<usize, XQError> {
    Err(XQError::Unsupported)
}

#[cfg(not(any(unix, windows)))]
pub fn query_osc_buffer<'b, MS: Into<u64> + Copy>(
    _query: &str,
    _buffer: &'b mut [u8],
    _timeout_ms: MS,
) -> Result<&'b [u8], XQError> {
    Err(XQError::Unsupported)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn wait_for_input<MS: Into<u64>>(fd: BorrowedFd<'_>, timeout_ms: MS) -> Result<i32, Errno> {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

    let poll_fd = PollFd::new(fd, PollFlags::POLLIN);
    let timeout = PollTimeout::try_from(timeout_ms.into()).map_err(|_| Errno::EOVERFLOW)?;

    poll(&mut [poll_fd], timeout)
}

// On MacOS, we need to use the `select` instead of `poll` because it doesn't support poll with tty:
//
// https://github.com/tokio-rs/mio/issues/1377
#[cfg(all(unix, target_os = "macos"))]
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
    let dur = Duration::from_millis(timeout_ms.into());
    let timeout_s = dur.as_secs() as _;
    let timeout_us = dur.subsec_micros() as _;
    let mut tv = TimeVal::new(timeout_s, timeout_us);

    select(
        fd.as_raw_fd() + 1,
        Some(&mut fd_set),
        None,
        None,
        Some(&mut tv),
    )
}

#[cfg(test)]
mod tests {
    use super::{scan_osc, OscScan};

    fn found(buf: &[u8]) -> Option<&[u8]> {
        match scan_osc(buf) {
            OscScan::Found { start, end } => Some(&buf[start..=end]),
            _ => None,
        }
    }

    #[test]
    fn bel_terminated_osc_then_fence() {
        // ESC ] 11 ; rgb:0000/0000/0000 BEL   then DSR reply ESC [ 0 n
        let buf = b"\x1b]11;rgb:1212/3434/5656\x07\x1b[0n";
        let payload = found(buf).expect("should find OSC payload");
        // payload excludes leading ESC, includes terminating BEL
        assert_eq!(payload, b"]11;rgb:1212/3434/5656\x07");
    }

    #[test]
    fn st_terminated_osc_then_fence() {
        // ST = ESC \ ; the terminating ESC is the OSC end
        let buf = b"\x1b]11;rgb:1212/3434/5656\x1b\\\x1b[0n";
        let payload = found(buf).expect("should find OSC payload");
        assert_eq!(payload, b"]11;rgb:1212/3434/5656\x1b");
    }

    #[test]
    fn osc_payload_containing_n_is_found() {
        // The OSC payload itself contains 'n' (e.g. `]11;none`); the scanner must
        // not mistake that 'n' for the DSR fence and return NotOsc early.
        let buf = b"\x1b]11;none\x07\x1b[0n";
        let payload = found(buf).expect("OSC payload with 'n' should still be found");
        assert_eq!(payload, b"]11;none\x07");
    }

    #[test]
    fn fence_answered_first_is_not_osc() {
        // Terminal doesn't support the query: only the DSR reply arrives.
        let buf = b"\x1b[0n";
        assert!(matches!(scan_osc(buf), OscScan::NotOsc));
    }

    #[test]
    fn incomplete_needs_more() {
        let buf = b"\x1b]11;rgb:1212/34";
        assert!(matches!(scan_osc(buf), OscScan::NeedMore));
    }

    #[test]
    fn empty_needs_more() {
        assert!(matches!(scan_osc(b""), OscScan::NeedMore));
    }
}
