/// xterm-query error type
#[derive(thiserror::Error, Debug)]
pub enum XQError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("UTF8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Wrong answer format: {0}")]
    WrongFormat(String),
    #[error("Timeout waiting for xterm")]
    Timeout,
    #[error("Terminal error code: {0}")]
    TerminalError(i64),
    #[error("Nix error: {0}")]
    #[cfg(unix)]
    NixError(#[from] nix::errno::Errno),
    #[error("Not an OSC response")]
    NotAnOSCResponse,
    #[error("Provided buffer is too small")]
    BufferOverflow,
    #[error("Unsupported platform")]
    Unsupported,
}
