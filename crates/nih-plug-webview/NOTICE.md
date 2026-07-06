# NOTICE

This crate is a vendored fork of
[`nih-plug-webview`](https://github.com/httnn/nih-plug-webview) by
Max Huttunen, used under the ISC license (see `LICENSE`).

Local modifications:

- Pinned `nih_plug` and `baseview` to the exact git revisions used by the
  rest of this workspace so editor and plugin share one set of types.
- The whole library is compiled only on Windows and macOS
  (`#![cfg(any(windows, target_os = "macos"))]`); embedding a webview in a
  host-owned plugin window is not supported by wry on Linux, where the
  plugins fall back to their egui editors.
