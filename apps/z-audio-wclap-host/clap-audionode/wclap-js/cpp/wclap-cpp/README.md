# WCLAP C++ helpers

## What is a WCLAP?

A WCLAP is a CLAP module compiled to WebAssembly (`wasm32` or `wasm64`).  It must:

* export `clap_entry` (an integer which is secretly `clap_plugin_entry *`)
* export `malloc()` or something like it (`void * malloc(size_t)`)
* import memory (must be shared) or export memory (may be shared) - if it does both, hosts should ignore the export.

The `wasm32` architecture is the most likely, but hosts should still check the integer type of `clap_entry` (and signature of `malloc`) to be sure.  There's not really an advantage to `wasm64` unless you're also using 64-bit memory, and support for that isn't universal.

If the WCLAP is a directory (or when fetched via URL, a `.tar.gz` which expands into a directory) then the actual binary should be `./module.wasm`.

The WCLAP may import WASI, and hosts should provide this where possible.  If the plugin is a directory, that entire directory should be available through the WASI filesystem (and pointed to in `clap_entry.init()`), but not necessarily at the top-level.

## C++ API

This repo includes the C++ header `wclap/wclap.hpp` which defines WCLAP equivalents to all the CLAP types.  These have the (function-)pointers replaced, so (as long as the host architecture is little-endian) they can be copied bitwise from the corresponding CLAP types inside the WASM memory.

These structures can't contain pointers, since those will all be specified relative to the WASM memory, not the host memory.  Similarly, they can't contain function pointers (a core part of the CLAP API), so these are represented by `Pointer<>` and `Function<>` wrappers respectively. 

(This is why it's in C++, sorry C/Rust folks.  The `Pointer<>`/`Function<>` templates make this _so_ much nicer.  If it's possible to write something similar in other languages, I'm not the person to do it.)

### `Instance` interface

To actually set/get `Pointer<>`s, or call `Function<>`s, you need an `Instance`.  This defined in `wclap/instance.hpp`, and _mostly_ forwards directly to an underlying implementation, with the exception of:
* `instance.init()` -> `impl.init32()` or `impl.init64()` as appropriate
* `instance.get()`/`.set()` -> forward to `.getArray()`/`.setArray()` with length 1

The idea is to abstract "a WCLAP running in some unknown WASM engine", making it easier to write a host and swap the WASM engine out later.  WASI support is up to the `Instance`, and there may be some implementation-specific config there:

![wclap-cpp overview diagram](doc/wclap-cpp-outline.png)

Host functions can only be registered before calling `Instance::init()`.  This will produce different `Function<>` values when registered on different `Instance`s, so they must be stored per-instance.

If a WCLAP spawns a new thread, the `Instance` should handle this internally, and then inform you about it by a callback on that new thread.  A thread spawned by one `Instance` will not be able to call methods on other `Instance`s.
