//! Webview editors for the native VST3/CLAP plugins.
//!
//! Reuses each plugin's WebCLAP UI (`crates/z-audio-webclap-*/ui/`)
//! verbatim inside a wry webview (the same engine Tauri uses), attached to
//! the host's plugin window via the vendored `nih_plug_webview` adapter.
//! The UI kit's `connect()` detects the wry IPC bridge and switches from
//! WebCLAP binary postMessage to JSON messages:
//!
//! ```text
//! UI -> plugin   {"type":"ready"}
//!                {"type":"set","id":<web id>,"value":<web value>}
//!                {"type":"bin","data":"<base64>"}
//! plugin -> UI   {"type":"params","values":{"<web id>":<web value>,…}}
//!                {"type":"bin","data":"<base64>"}
//! ```
//!
//! "Web values" are the same plain values the WebCLAP builds use; each
//! [`ParamMapping`] carries a pair of conversions to/from the native
//! parameter's plain value for the rare cases where units differ.
//!
//! The `bin` envelope carries the raw binary packets a WebCLAP UI would
//! send over `clap.webview/3` (base64ed, since wry's IPC is string-only).
//! Plugins that speak a binary protocol (the sampler's `ZSMP` messages)
//! pass a [`BinaryMessageHandler`] to
//! [`create_webview_editor_with_messages`]; a UI `ready` is forwarded to
//! that handler as the WebCLAP ready packet (`\x65ready`) so plugin code
//! can reuse its `on_ui_message` dispatch verbatim.
//!
//! Only compiled on Windows and macOS — wry cannot embed into host-owned
//! windows on Linux, where the plugins keep their egui editors.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use nih_plug::prelude::ParamPtr;
use serde_json::Value as JsonValue;

#[cfg(any(windows, target_os = "macos"))]
use {
    nih_plug::prelude::Editor,
    nih_plug_webview::{HTMLSource, WebViewEditor},
    serde_json::{json, Map, Value},
    std::collections::HashMap,
    std::sync::Mutex,
};

/// One web-UI parameter bound to one native parameter.
pub struct ParamMapping {
    /// The numeric param id the web UI uses (matches the WebCLAP build).
    pub web_id: u32,
    pub ptr: ParamPtr,
    /// Web value -> native plain value. Usually the identity from [`map`].
    pub to_plain: fn(f64) -> f32,
    /// Native plain value -> web value.
    pub to_web: fn(f32) -> f64,
}

/// Identity mapping — the web UI and the native param share units.
pub fn map(web_id: u32, ptr: ParamPtr) -> ParamMapping {
    ParamMapping {
        web_id,
        ptr,
        to_plain: |v| v as f32,
        to_web: |v| v as f64,
    }
}

/// Mapping with custom unit conversions.
pub fn map_scaled(
    web_id: u32,
    ptr: ParamPtr,
    to_plain: fn(f64) -> f32,
    to_web: fn(f32) -> f64,
) -> ParamMapping {
    ParamMapping {
        web_id,
        ptr,
        to_plain,
        to_web,
    }
}

/// UI -> plugin binary message callback, running on the editor (GUI)
/// thread. `reply` sends plugin -> UI binary packets back through the
/// same `{"type":"bin"}` envelope.
pub type BinaryMessageHandler = std::sync::Arc<dyn Fn(&[u8], &mut dyn FnMut(&[u8])) + Send + Sync>;

/// The WebCLAP scaffold's `ready` packet (CBOR text "ready"), forwarded to
/// the [`BinaryMessageHandler`] when the web UI reports ready so plugin
/// code can reuse its WebCLAP `on_ui_message` dispatch unchanged.
pub const READY_PACKET: &[u8] = b"\x65ready";

/// Wraps raw binary bytes in the `{"type":"bin","data":"<base64>"}` JSON
/// envelope used in both directions over the wry IPC bridge.
pub fn encode_bin_envelope(bytes: &[u8]) -> JsonValue {
    let mut map = serde_json::Map::with_capacity(2);
    map.insert("type".into(), JsonValue::from("bin"));
    map.insert("data".into(), JsonValue::from(BASE64.encode(bytes)));
    JsonValue::Object(map)
}

/// Extracts the binary payload from a `{"type":"bin","data":…}` envelope.
/// Returns `None` for any other message (or undecodable base64).
pub fn decode_bin_envelope(value: &JsonValue) -> Option<Vec<u8>> {
    if value.get("type").and_then(JsonValue::as_str) != Some("bin") {
        return None;
    }
    let data = value.get("data").and_then(JsonValue::as_str)?;
    BASE64.decode(data).ok()
}

/// Inlines a WebCLAP `ui/` bundle (index.html + styles.css + zui.js +
/// main.js) into one self-contained HTML string. The two tags below are
/// written identically in every Z Audio UI, and `main.js`'s single
/// `import … from "./zui.js";` line is dropped after `zui.js` (with its
/// `export` keywords stripped) is prepended into the same module scope.
pub fn inline_ui_html(index_html: &str, styles_css: &str, zui_js: &str, main_js: &str) -> String {
    let zui_inlined = zui_js
        .replace("\nexport function ", "\nfunction ")
        .replace("\nexport const ", "\nconst ");
    let main_without_import: String = main_js
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!("{zui_inlined}\n{main_without_import}");
    index_html
        .replace(
            "<link rel=\"stylesheet\" href=\"./styles.css\" />",
            &format!("<style>\n{styles_css}\n</style>"),
        )
        .replace(
            "<script type=\"module\" src=\"./main.js\"></script>",
            &format!("<script type=\"module\">\n{script}\n</script>"),
        )
}

/// Builds the shared-UI webview editor. `html` is the inlined page (see
/// [`inline_ui_html`]; plugins cache it in a `OnceLock<String>` and leak
/// once so the editor can hold `&'static str`).
#[cfg(any(windows, target_os = "macos"))]
pub fn create_webview_editor(
    html: &'static str,
    size: (u32, u32),
    mappings: Vec<ParamMapping>,
) -> Option<Box<dyn Editor>> {
    create_webview_editor_with_messages(html, size, mappings, None)
}

/// Like [`create_webview_editor`], plus an optional binary message channel
/// for UIs that speak a WebCLAP-style binary protocol alongside the param
/// sync (see the module docs' `bin` envelope).
#[cfg(any(windows, target_os = "macos"))]
pub fn create_webview_editor_with_messages(
    html: &'static str,
    size: (u32, u32),
    mappings: Vec<ParamMapping>,
    on_binary: Option<BinaryMessageHandler>,
) -> Option<Box<dyn Editor>> {
    // Last web value pushed per param, so the frame callback only sends
    // actual changes (host automation, preset loads, other UIs).
    let sent: Mutex<HashMap<u32, f64>> = Mutex::new(HashMap::new());

    let editor = WebViewEditor::new(HTMLSource::String(html), size)
        .with_background_color((9, 13, 19, 255))
        .with_event_loop(move |ctx, setter, _window| {
            let mut sent = match sent.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            while let Ok(event) = ctx.next_event() {
                match event.get("type").and_then(Value::as_str) {
                    Some("ready") => {
                        // Force a full snapshot below.
                        sent.clear();
                        // Forward the WebCLAP ready packet so binary
                        // plugins push their initial status.
                        if let Some(handler) = on_binary.as_ref() {
                            handler(READY_PACKET, &mut |bytes| {
                                ctx.send_json(encode_bin_envelope(bytes));
                            });
                        }
                    }
                    Some("bin") => {
                        let (Some(handler), Some(bytes)) =
                            (on_binary.as_ref(), decode_bin_envelope(&event))
                        else {
                            continue;
                        };
                        handler(&bytes, &mut |reply| {
                            ctx.send_json(encode_bin_envelope(reply));
                        });
                    }
                    Some("set") => {
                        let (Some(id), Some(value)) = (
                            event.get("id").and_then(Value::as_u64),
                            event.get("value").and_then(Value::as_f64),
                        ) else {
                            continue;
                        };
                        let Some(mapping) = mappings.iter().find(|m| m.web_id == id as u32) else {
                            continue;
                        };
                        let plain = (mapping.to_plain)(value);
                        // SAFETY: the ParamPtr comes from this plugin's own
                        // Params object, which outlives the editor.
                        unsafe {
                            let normalized = mapping.ptr.preview_normalized(plain);
                            setter.raw_context.raw_begin_set_parameter(mapping.ptr);
                            setter
                                .raw_context
                                .raw_set_parameter_normalized(mapping.ptr, normalized);
                            setter.raw_context.raw_end_set_parameter(mapping.ptr);
                        }
                        // Remember what the UI itself set so the diff pass
                        // doesn't echo it straight back.
                        sent.insert(id as u32, value);
                    }
                    _ => {}
                }
            }

            // Diff pass: push any param whose value moved since last frame.
            let mut changed = Map::new();
            for mapping in &mappings {
                // SAFETY: see above.
                let plain = unsafe { mapping.ptr.unmodulated_plain_value() };
                let web = (mapping.to_web)(plain);
                let stale = sent
                    .get(&mapping.web_id)
                    .map(|prev| (prev - web).abs() > 1.0e-9)
                    .unwrap_or(true);
                if stale {
                    changed.insert(mapping.web_id.to_string(), json!(web));
                    sent.insert(mapping.web_id, web);
                }
            }
            if !changed.is_empty() {
                ctx.send_json(json!({ "type": "params", "values": changed }));
            }
        });

    Some(Box::new(editor))
}

/// Convenience macro: inline a WebCLAP ui/ bundle at compile time, cache
/// the composed page, and build the editor. `$ui` is the path (relative to
/// the calling crate's `src/`) to the sibling webclap crate's `ui` dir.
#[macro_export]
macro_rules! webview_editor_from_ui {
    ($ui:literal, $size:expr, $mappings:expr) => {{
        static HTML: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
        let html = HTML.get_or_init(|| {
            $crate::inline_ui_html(
                include_str!(concat!($ui, "/index.html")),
                include_str!(concat!($ui, "/styles.css")),
                include_str!(concat!($ui, "/zui.js")),
                include_str!(concat!($ui, "/main.js")),
            )
        });
        $crate::create_webview_editor(html.as_str(), $size, $mappings)
    }};
}

#[cfg(test)]
mod tests {
    use super::{decode_bin_envelope, encode_bin_envelope, inline_ui_html};

    #[test]
    fn bin_envelope_round_trips() {
        let payload: Vec<u8> = (0..=255u8).collect();
        let envelope = encode_bin_envelope(&payload);
        assert_eq!(envelope.get("type").unwrap(), "bin");
        assert_eq!(decode_bin_envelope(&envelope), Some(payload));
    }

    #[test]
    fn bin_envelope_ignores_foreign_messages() {
        assert_eq!(
            decode_bin_envelope(&serde_json::json!({"type": "set", "id": 1, "value": 0.5})),
            None
        );
        assert_eq!(
            decode_bin_envelope(&serde_json::json!({"type": "bin", "data": "@@not-base64@@"})),
            None
        );
        assert_eq!(
            decode_bin_envelope(&serde_json::json!({"type": "bin"})),
            None
        );
    }

    #[test]
    fn bin_envelope_survives_json_serialization() {
        // The envelope crosses the IPC bridge as a JSON string; make sure a
        // serialize/deserialize round trip preserves the payload.
        let payload = b"ZSMP\x04\x01\x3c\x64".to_vec();
        let text = encode_bin_envelope(&payload).to_string();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(decode_bin_envelope(&parsed), Some(payload));
    }

    #[test]
    fn inlining_composes_one_self_contained_page() {
        let html = "<head><link rel=\"stylesheet\" href=\"./styles.css\" /></head>\n<body><script type=\"module\" src=\"./main.js\"></script></body>";
        let css = ".x { color: red; }";
        let zui = "\nexport function connect() {}\nexport const fmt = 1;";
        let main = "import { connect, fmt } from \"./zui.js\";\nconnect();";
        let out = inline_ui_html(html, css, zui, main);
        assert!(out.contains("<style>\n.x { color: red; }\n</style>"));
        assert!(out.contains("function connect() {}"));
        assert!(out.contains("const fmt = 1;"));
        assert!(!out.contains("import {"));
        assert!(!out.contains("export "));
        assert!(out.contains("connect();"));
    }
}
