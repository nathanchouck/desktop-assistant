# Everything SDK (vendored)

The official [Everything](https://www.voidtools.com/) 1.4 SDK client source by
David Carpenter (voidtools), MIT licensed (the notice is at the top of each
file). Compiled straight into the app on Windows by `build.rs`, so there is no
`Everything64.dll` to ship.

Taken from the vendored copy in the `everything-sdk-sys` 0.0.3+2 crate,
because voidtools.com wasn't reachable from the build environment. The only
change is the file name `ipc/Everything_IPC.h` (was `everything_ipc.h`), to
match the `#include` in `Everything.c`.

Our Rust bindings live in `src/platform/win/everything.rs`.
