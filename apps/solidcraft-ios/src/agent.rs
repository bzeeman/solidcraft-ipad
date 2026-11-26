//! The agent chat on the iPad. iPadOS starts no other programs, so the chat's endpoint agents
//! (an OpenAI- or Anthropic-compatible URL: LM Studio or Ollama on a Mac on the same network, or
//! a hosted model) cannot reach SolidCraft's tools through the stdio helper the desktop starts
//! (`solidcraft-cli mcp --connect`). Here the same MCP server runs inside the app instead, on the
//! chat's worker thread, and each tool call reaches the window through the chat's gate as a
//! control request: the same approvals, limits and one undo step as on the desktop.
//!
//! The chat's model requests go out through [`http`], as the desktop app's do.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use solidcraft_control::ControlRequest;
use solidcraft_ui_egui::agent_chat::backend::{HttpRequest, InProcessStart, InProcessStarter, InProcessTools};

/// SolidCraft's MCP server, held on the chat's worker thread.
struct Tools(solidcraft_mcp::Server);

impl InProcessTools for Tools {
    fn handle_line(&mut self, line: &str) -> Option<String> {
        self.0.handle_line(line)
    }
}

/// The server for one agent connection: its calls go to `tx`, where the chat's gate reads them,
/// and stop waiting when the chat's Stop flag is set.
fn start(tx: Sender<ControlRequest>) -> InProcessStart {
    Box::new(move |cancel: Arc<AtomicBool>| {
        let backend = solidcraft_mcp::Channel::new(tx).with_cancel(cancel);
        Box::new(Tools(solidcraft_mcp::Server::new(Box::new(backend)))) as Box<dyn InProcessTools>
    })
}

/// The hook the chat starts its in-process tools with.
pub fn starter() -> InProcessStarter {
    Box::new(start)
}

/// HTTP for the chat's endpoint agents: to the URL the user configured, which may be plain HTTP
/// on the local network (a Mac running LM Studio or Ollama). No redirects, so a key never follows
/// one elsewhere, and at most 16 MB back. Certificates are checked by the system's verifier.
pub fn http(r: &HttpRequest) -> Result<(u16, Vec<u8>), String> {
    use std::time::Duration;
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(r.timeout_s.clamp(1, 3600))))
        .timeout_connect(Some(Duration::from_secs(10)))
        .user_agent("SolidCraft")
        .max_redirects(0)
        .build()
        .into();
    let mut response = if r.method == "POST" {
        let mut req = agent.post(&r.url);
        for (k, v) in &r.headers {
            req = req.header(k, v);
        }
        req.send(r.body.as_deref().unwrap_or_default())
    } else {
        let mut req = agent.get(&r.url);
        for (k, v) in &r.headers {
            req = req.header(k, v);
        }
        req.call()
    }
    .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(16 << 20).read_to_vec().map_err(|e| e.to_string())?;
    Ok((status, body))
}
