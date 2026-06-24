//! Windows backend for terminal querying.
//!
//! The Unix backend writes the query to stdout and reads the reply from
//! `/dev/tty`, waiting with `poll`/`select`. On Windows the equivalent is to
//! write the query to stdout, then read the reply from the console input
//! (`CONIN$`) after waiting on its handle with `WaitForSingleObject`.
//!
//! Two Windows specifics matter:
//! - The reply only arrives as a byte stream when the console input is in
//!   `ENABLE_VIRTUAL_TERMINAL_INPUT` mode, so we set it (and clear line, echo,
//!   and processed input) for the duration of the query, restoring the
//!   previous mode on drop.
//! - As on Unix, the caller is expected to have the terminal in raw mode and
//!   to issue the query at a moment it isn't otherwise reading input.
//!
//! Limitation: `WaitForSingleObject` on the console input handle is signaled
//! by non-character events too (focus, mouse, buffer-resize), which under
//! VT-input mode may be delivered as escape sequences. A stray such event
//! arriving in the query window can produce a wrong or empty reply; callers
//! treat an unparsable reply as "unsupported" and degrade gracefully rather
//! than hang. Filtering to key events via `ReadConsoleInputW` would harden
//! this further.

use {
    crate::{scan_osc, OscScan, XQError},
    std::io::{self, Write},
    windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{CreateFileW, ReadFile},
        System::{
            Console::{GetConsoleMode, SetConsoleMode},
            Threading::WaitForSingleObject,
        },
    },
};

// Stable Win32 ABI constants, defined locally to stay independent of
// windows-sys module reorganizations between versions.
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;

const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
const ENABLE_LINE_INPUT: u32 = 0x0002;
const ENABLE_ECHO_INPUT: u32 = 0x0004;
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

const WAIT_OBJECT_0: u32 = 0x0000_0000;
const WAIT_TIMEOUT: u32 = 0x0000_0102;

/// A `CONIN$` handle put into VT-input mode for the duration of a query, with
/// the previous console mode restored on drop.
struct ConsoleInput {
    handle: HANDLE,
    prev_mode: u32,
}

impl ConsoleInput {
    fn open() -> Result<Self, XQError> {
        // "CONIN$" gives us the console input even if stdin is redirected,
        // mirroring the Unix backend's use of /dev/tty.
        let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(XQError::IO(io::Error::last_os_error()));
        }
        let mut prev_mode: u32 = 0;
        if unsafe { GetConsoleMode(handle, &mut prev_mode) } == 0 {
            let err = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(XQError::IO(err));
        }
        let new_mode = (prev_mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
            & !ENABLE_LINE_INPUT
            & !ENABLE_ECHO_INPUT
            & !ENABLE_PROCESSED_INPUT;
        if unsafe { SetConsoleMode(handle, new_mode) } == 0 {
            let err = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(XQError::IO(err));
        }
        Ok(Self { handle, prev_mode })
    }

    /// Wait up to `timeout_ms` for input to be available.
    /// Returns `Ok(true)` if input is ready, `Ok(false)` on timeout.
    fn wait(&self, timeout_ms: u64) -> Result<bool, XQError> {
        let ms = u32::try_from(timeout_ms).unwrap_or(u32::MAX);
        match unsafe { WaitForSingleObject(self.handle, ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(XQError::IO(io::Error::last_os_error())),
        }
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, XQError> {
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                self.handle,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &mut read,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(XQError::IO(io::Error::last_os_error()));
        }
        Ok(read as usize)
    }
}

impl Drop for ConsoleInput {
    fn drop(&mut self) {
        unsafe {
            SetConsoleMode(self.handle, self.prev_mode);
            CloseHandle(self.handle);
        }
    }
}

fn write_query(query: &str) -> Result<(), XQError> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "{}", query)?;
    stdout.flush()?;
    Ok(())
}

/// Query the terminal, assuming it is in raw mode. Returns the number of bytes
/// read into `buffer`.
pub fn query_buffer<MS: Into<u64>>(
    query: &str,
    buffer: &mut [u8],
    timeout_ms: MS,
) -> Result<usize, XQError> {
    // Put the console into VT-input mode BEFORE emitting the query: the terminal
    // may reply within microseconds, and a reply arriving before the mode switch
    // would not be delivered as the byte stream we read here.
    let con = ConsoleInput::open()?;
    write_query(query)?;
    if con.wait(timeout_ms.into())? {
        con.read(buffer)
    } else {
        Err(XQError::Timeout)
    }
}

/// Query the terminal for an OSC response, assuming it is in raw mode.
///
/// Mirrors the Unix backend: the query is followed by a DSR ("fence") request
/// so terminals that don't support the query can be detected by answering the
/// Status Report first. The returned slice excludes the leading ESC and
/// everything before it, and ends at (and includes) the OSC terminator.
pub fn query_osc_buffer<'b, MS: Into<u64> + Copy>(
    query: &str,
    buffer: &'b mut [u8],
    timeout_ms: MS,
) -> Result<&'b [u8], XQError> {
    const ESC: char = '\x1b';
    // Switch the console into VT-input mode BEFORE emitting the query (see
    // query_buffer), so a fast reply is captured as bytes.
    let con = ConsoleInput::open()?;
    // query, then the DSR fence
    {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        write!(stdout, "{}", query)?;
        write!(stdout, "{ESC}[5n")?;
        stdout.flush()?;
    }
    let mut filled = 0;
    while filled < buffer.len() {
        if !con.wait(timeout_ms.into())? {
            return Err(XQError::Timeout);
        }
        let bytes_read = con.read(&mut buffer[filled..])?;
        if bytes_read == 0 {
            return Err(XQError::NotAnOSCResponse);
        }
        filled += bytes_read;
        match scan_osc(&buffer[..filled]) {
            OscScan::Found { start, end } => return Ok(&buffer[start..=end]),
            OscScan::NotOsc => return Err(XQError::NotAnOSCResponse),
            OscScan::NeedMore => {}
        }
    }
    Err(XQError::BufferOverflow)
}
