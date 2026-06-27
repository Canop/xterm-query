[![MIT][s2]][l2] [![Latest Version][s1]][l1] [![docs][s3]][l3] [![Chat on Miaou][s4]][l4]

[s1]: https://img.shields.io/crates/v/xterm-query.svg
[l1]: https://crates.io/crates/xterm-query

[s2]: https://img.shields.io/badge/license-MIT-blue.svg
[l2]: LICENSE

[s3]: https://docs.rs/xterm-query/badge.svg
[l3]: https://docs.rs/xterm-query/

[s4]: https://miaou.dystroy.org/static/shields/room.svg
[l4]: https://miaou.dystroy.org/3


<!-- cradoc start -->
Query the terminal by writing an escape sequence and reading the reply.

The terminal must be in raw mode (otherwise reads block on a newline), and
the query should be issued when nothing else is reading terminal input.

The provided example in examples/kitty demonstrates querying the terminal to
know whether the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
is supported, and manages entering and leaving raw mode.

# Platform support

This crate supports Linux, MacOS, and Windows. When a platform is not supported, the query
functions return `XQError::Unsupported`.

Unix reads the reply from `/dev/tty` using `poll`/`select`. Windows reads it
from the console input (`CONIN$`) after switching it to
`ENABLE_VIRTUAL_TERMINAL_INPUT` for the duration of the query.

## Windows limitation

The Windows wait wakes on any console input event, including focus, mouse,
and buffer-resize events, which under VT-input mode may be delivered as
escape sequences. If such an event arrives in the query window, the reply
can be wrong or empty, so callers should treat an unparsable reply as
"unsupported" and degrade gracefully. Issue queries while the terminal is
otherwise quiet (e.g. at startup, before an input loop begins).

<!-- cradoc end -->


