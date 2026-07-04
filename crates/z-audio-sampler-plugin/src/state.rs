//! Shared (non-realtime) state for the "Load Sample..." workflow: the
//! editor (UI thread) writes to these, `process`/`reinit` (audio/setup
//! thread) read them. Never touched from inside the realtime `process` loop
//! beyond a non-blocking `try_lock`.

use std::sync::Arc;

use z_audio_synth::SamplerBank;

/// User-facing status shown in the editor.
#[derive(Clone, Debug, Default)]
pub enum LoadStatus {
    #[default]
    Empty,
    Loaded {
        file_name: String,
    },
    Missing {
        path: String,
    },
    Error {
        message: String,
    },
}

impl LoadStatus {
    pub fn label(&self) -> String {
        match self {
            LoadStatus::Empty => "No sample loaded (using built-in demo)".to_string(),
            LoadStatus::Loaded { file_name } => format!("Loaded: {file_name}"),
            LoadStatus::Missing { path } => format!("Missing file: {path}"),
            LoadStatus::Error { message } => format!("Load failed: {message}"),
        }
    }
}

/// A bank change requested by the editor, picked up by `process` on the
/// next block via a non-blocking `try_lock`.
pub enum BankUpdate {
    Loaded(Arc<SamplerBank>),
    Cleared,
}
