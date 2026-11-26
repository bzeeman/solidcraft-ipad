//! The MCP tools served in the app's own process: each control-channel call becomes a
//! [`ControlRequest`] sent straight to the window, the way the loopback server hands one over,
//! but with no socket, token or second process. For a platform that cannot start the stdio
//! helper (`solidcraft-cli mcp --connect`), such as iPadOS, where the agent chat's own tool loop
//! then runs this server on its worker thread.

use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use solidcraft_control::ControlRequest;
use solidcraft_engine::schema::ParamCheck;

use super::Backend;

/// How long a call waits for the window by default: as long as the chat waits for a tool, since a
/// change may wait on the user's Allow.
const DEFAULT_WAIT: Duration = Duration::from_secs(320);
/// How often a waiting call looks at its cancel flag.
const POLL: Duration = Duration::from_millis(40);

/// A backend whose calls go to the window through `tx` (the agent chat's gate reads them).
pub struct Channel {
    tx: Sender<ControlRequest>,
    wait: Duration,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Channel {
    pub fn new(tx: Sender<ControlRequest>) -> Self {
        Self { tx, wait: DEFAULT_WAIT, cancel: None }
    }

    /// How long one call waits for its reply.
    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// A flag that, once set, stops the call waiting (the user pressed Stop). The request may
    /// still run in the window: the reply says so.
    pub fn with_cancel(mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// `params` with the agent's folders, as the loopback bridge sends them (see `Remote`).
    fn scoped(mut params: Value) -> Value {
        if let (Some(scope), Some(o)) = (solidcraft_engine::io::vfs::current_scope(), params.as_object_mut()) {
            o.insert("allow_paths".into(), json!(scope.roots().iter().map(|r| r.to_string_lossy()).collect::<Vec<_>>()));
            if !scope.design_roots().is_empty() {
                o.insert("design_folders".into(), json!(scope.design_roots().iter().map(|r| r.to_string_lossy()).collect::<Vec<_>>()));
            }
        }
        params
    }

    fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        let (req, reply) = ControlRequest::new(method, Self::scoped(params));
        self.tx.send(req).map_err(|_| "the SolidCraft window is gone".to_string())?;
        let deadline = Instant::now() + self.wait;
        loop {
            if self.cancel.as_ref().is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
                return Err(format!("{} cancelled by the user while waiting for the window", super::STILL_RUNNING));
            }
            let left = deadline.saturating_duration_since(Instant::now()).min(POLL);
            match reply.recv_timeout(left) {
                Ok(v) => return solidcraft_control::client::outcome(&v),
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{} the window did not answer {method} within {} s", super::STILL_RUNNING, self.wait.as_secs()));
                }
                Err(RecvTimeoutError::Disconnected) => return Err(format!("the window dropped {method} without an answer")),
            }
        }
    }
}

impl Backend for Channel {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            // What the loopback server answers itself, for every connection.
            "control.capabilities" => Ok(json!({"param_check": [ParamCheck::Off.name()], "plugin_tools": true})),
            // One call at a time on this thread: nothing else of ours can be running.
            "control.running" => Ok(json!({"running": []})),
            "control.pending" => Ok(json!({"pending": null})),
            "control.cancel" => Err("nothing of this session is running to cancel".into()),
            _ => self.send(method, params),
        }
    }
    fn supports_plugin_tools(&mut self) -> bool {
        true
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        "inside the SolidCraft app (the user watches the viewport)".into()
    }
    fn app_options(&mut self) -> Result<Option<Value>, String> {
        match self.send("mcp.options", json!({})) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.starts_with("unknown method") || e.contains("is not available in the agent chat") => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call reaches the window as a request and its answer comes back as the result; an error
    /// answer is an error, and a window that never answers ends the wait with "still running".
    #[test]
    fn calls_go_to_the_window_and_back() {
        let (tx, rx) = std::sync::mpsc::channel::<ControlRequest>();
        let window = std::thread::spawn(move || {
            for req in rx {
                match req.method.as_str() {
                    "engine.execute" => {
                        let _ = req.reply.send(json!({"ok": true, "result": {"ran": req.params["command"]}}));
                    }
                    "ui.view" => {} // never answered
                    _ => {
                        let _ = req.reply.send(json!({"ok": false, "error": format!("unknown method {}", req.method)}));
                    }
                }
            }
        });
        let mut b = Channel::new(tx).with_wait(Duration::from_millis(200));
        assert_eq!(b.call("engine.execute", json!({"command": "solid.box"})), Ok(json!({"ran": "solid.box"})));
        assert!(b.call("document.nothing", json!({})).is_err_and(|e| e.starts_with("unknown method")));
        assert!(b.call("ui.view", json!({})).is_err_and(|e| e.starts_with(super::super::STILL_RUNNING)));
        assert_eq!(b.call("control.running", json!({})), Ok(json!({"running": []})));
        assert_eq!(b.app_options(), Ok(None));
        drop(b);
        let _ = window.join();
    }
}
