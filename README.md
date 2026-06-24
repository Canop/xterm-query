[![MIT][s2]][l2] [![Latest Version][s1]][l1] [![docs][s3]][l3] [![Chat on Miaou][s4]][l4]

[s1]: https://img.shields.io/crates/v/xterm-query.svg
[l1]: https://crates.io/crates/xterm-query

[s2]: https://img.shields.io/badge/license-MIT-blue.svg
[l2]: LICENSE

[s3]: https://docs.rs/xterm-query/badge.svg
[l3]: https://docs.rs/xterm-query/

[s4]: https://miaou.dystroy.org/static/shields/room.svg
[l4]: https://miaou.dystroy.org/3


Low level library to query the terminal with a CSI sequence and get the result as a string.

Notes:

- the terminal must already be in raw mode
- the query should be issued while nothing else is reading terminal input
- Windows is supported, reading the reply from the console input (`CONIN$`) after switching it to `ENABLE_VIRTUAL_TERMINAL_INPUT` for the duration of the query. Caveat: the Windows wait can wake on non-character console events (focus, mouse, buffer-resize), so a reply may occasionally be wrong or empty — treat an unparsable reply as "unsupported", and issue queries while the terminal is quiet (e.g. at startup)

The provided example in examples/kitty demonstrates querying the terminal to know whether the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) is supported, and manages entering and leaving raw mode.

If you think you might use this crate but are unsure, don't hesitate to come to Miaou: [![s4]][s4]

