# Vendored crates

`miniquad/` is miniquad 0.4.11 with two small fixes, marked "Snook patch" in the
source (used through `[patch.crates-io]` in the workspace `Cargo.toml`):

- `src/graphics/gl.rs`: stream buffers are re-specified ("orphaned") on every
  update. Without it, Intel graphics drivers on Windows make the 2D interface
  flicker or disappear.
- `src/native/windows.rs`: full screen uses the monitor the window is on and
  restores the window's previous place afterwards.
