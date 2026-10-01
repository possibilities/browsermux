//! UniFFI-owned boundary: no CEF pointers or borrowed Rust allocations cross it.
use browser_core::{BrowserCore, SessionWriter};
use std::sync::{Arc, Mutex};
uniffi::setup_scaffolding!();
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BridgeError {
    #[error("{message}")]
    Domain { message: String },
    #[error("core state lock is unavailable")]
    Unavailable,
}
fn err(e: impl std::fmt::Display) -> BridgeError {
    BridgeError::Domain {
        message: e.to_string(),
    }
}
#[derive(uniffi::Object)]
pub struct BrowserStore {
    inner: Mutex<BrowserCore>,
    recovery_notice: Option<String>,
    writer: Mutex<Option<SessionWriter>>,
}
#[uniffi::export]
impl BrowserStore {
    #[uniffi::constructor]
    pub fn open(data_root: String) -> Result<Arc<Self>, BridgeError> {
        let (mut core, notice) =
            BrowserCore::open_or_recover(std::path::Path::new(&data_root)).map_err(err)?;
        let writer = core.detach_persistence();
        Ok(Arc::new(Self {
            inner: Mutex::new(core),
            writer: Mutex::new(writer),
            recovery_notice: notice.map(|n| n.message),
        }))
    }
    #[uniffi::constructor]
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(BrowserCore::new()),
            writer: Mutex::new(None),
            recovery_notice: None,
        })
    }
    pub fn recovery_notice(&self) -> Option<String> {
        self.recovery_notice.clone()
    }
    pub fn snapshot_json(&self) -> Result<String, BridgeError> {
        let core = self.inner.lock().map_err(|_| BridgeError::Unavailable)?;
        serde_json::to_string(&core.snapshot()).map_err(err)
    }
    pub fn dispatch_json(&self, command: String) -> Result<String, BridgeError> {
        if command.len() > 65536 {
            return Err(err("command exceeds 64 KiB"));
        }
        let mut core = self.inner.lock().map_err(|_| BridgeError::Unavailable)?;
        let result = core.dispatch_json(&command).map_err(err)?;
        serde_json::to_string(&result).map_err(err)
    }
    pub fn flush(&self) -> Result<(), BridgeError> {
        let prepared = self
            .inner
            .lock()
            .map_err(|_| BridgeError::Unavailable)?
            .prepare_session()
            .map_err(err)?;
        let saved = {
            let mut guard = self.writer.lock().map_err(|_| BridgeError::Unavailable)?;
            match guard.as_mut() {
                Some(writer) => writer.save(&prepared).map_err(err)?,
                None => true,
            }
        };
        if saved {
            self.inner
                .lock()
                .map_err(|_| BridgeError::Unavailable)?
                .mark_session_saved(prepared.revision());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_dispatches_owned_commands_and_rejects_large_input() {
        let store = BrowserStore::in_memory();
        let before: serde_json::Value =
            serde_json::from_str(&store.snapshot_json().unwrap()).unwrap();
        let result: serde_json::Value = serde_json::from_str(
            &store
                .dispatch_json(r#"{"type":"split","axis":"left_right"}"#.into())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(before["panes"].as_array().unwrap().len(), 1);
        assert_eq!(result["snapshot"]["panes"].as_array().unwrap().len(), 2);
        assert!(store.dispatch_json(" ".repeat(65537)).is_err());
        store.flush().unwrap();
    }
}
