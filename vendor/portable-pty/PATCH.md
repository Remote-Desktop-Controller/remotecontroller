# portable-pty 0.9.0 — headless ConPTY

Source: official crates.io portable-pty 0.9.0, copied unchanged except the
CreatePseudoConsole flags in src/win/psuedocon.rs and the inverted BOOL check in
WinChildKiller::kill (src/win/mod.rs). MIT license retained.
Original psuedocon.rs SHA256 is recorded in docs/product/terminal-conpty-report.md.

The upstream flags request inherited cursor position and Win32 key records,
which require a GUI terminal emulator. The runtime is a headless UTF-8 VT stream.
The real Windows regression captured only ESC[6n and no shell progress. Flags=0
selects Microsoft's documented default, avoiding the unhandled cursor query and
preserving plain terminal input. Successful TerminateProcess was incorrectly
reported as an error (os error 0); it now reports success and accepts an already
exited handle as closed. Linux/macOS code is unchanged.

Regression: apps/local-daemon/tests/terminal.rs::real_pty_write_retry_resize_close_tree_and_durable_output.
Remove this patch only after an upstream configurable/default headless mode passes
the same native test, including input, resize, capture and child cleanup.
