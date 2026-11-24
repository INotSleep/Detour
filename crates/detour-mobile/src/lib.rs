//! Detour's core inside the iOS app. The app holds one `DetourCore`: calls go to the core as
//! they would over its socket (`vault_unlock`, `session_open`, ...: see `detour-daemon`'s
//! `rpc`), with their parameters and results as JSON, and what the core sends on its own
//! (events, terminal output, a session's end) comes back through `CoreListener`.

use std::sync::Arc;

use detour_ipc::{Frame, Message};

uniffi::setup_scaffolding!();

/// What the core tells the app without being asked.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    /// Something changed (`sessions.changed`, `locked`, ...); `data` is JSON.
    fn on_event(&self, name: String, data: String);
    /// Output of a terminal session, on the stream the app opened it with.
    fn on_data(&self, stream: u64, bytes: Vec<u8>);
    /// The end of a stream (`{"closed": true, "exitStatus": ...}`), as JSON.
    fn on_stream_end(&self, stream: u64, data: String);
}

/// A call the core refused: the core's error as JSON (`{"kind": "locked"}`, ...).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("{json}")]
    Failed { json: String },
}

#[derive(uniffi::Object)]
pub struct DetourCore {
    runtime: tokio::runtime::Runtime,
    core: Arc<detour_daemon::Embedded>,
}

#[uniffi::export]
impl DetourCore {
    /// Opens the core of the vault in `data_dir` (the app's own folder).
    #[uniffi::constructor]
    pub fn new(data_dir: String, listener: Arc<dyn CoreListener>) -> Result<Arc<Self>, CoreError> {
        let failed = |e: std::io::Error| CoreError::Failed { json: serde_json::json!({ "kind": "io", "message": e.to_string() }).to_string() };
        std::fs::create_dir_all(&data_dir).map_err(failed)?;
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("detour-core").build().map_err(failed)?;
        let core = runtime.block_on(async move {
            Arc::new(detour_daemon::Embedded::start(std::path::Path::new(&data_dir), move |frame| match frame {
                Frame::Data { stream, bytes } => listener.on_data(stream, bytes),
                Frame::Msg(Message::Event { name, data }) => listener.on_event(name, data.to_string()),
                Frame::Msg(Message::Stream { stream, data }) => listener.on_stream_end(stream, data.to_string()),
                _ => {}
            }))
        });
        Ok(Arc::new(Self { runtime, core }))
    }

    /// Calls the core: `params` and the result are JSON.
    pub async fn call(&self, method: String, params: String) -> Result<String, CoreError> {
        let params: serde_json::Value = if params.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(&params).map_err(|e| CoreError::Failed { json: serde_json::json!({ "kind": "badArguments", "message": e.to_string() }).to_string() })? };
        let core = self.core.clone();
        // On the core's own runtime: the call may start tasks that outlive it.
        let joined = self.runtime.spawn(async move { core.call(&method, params).await }).await;
        match joined {
            Ok(Ok(v)) => Ok(v.to_string()),
            Ok(Err(e)) => Err(CoreError::Failed { json: e.to_string() }),
            Err(e) => Err(CoreError::Failed { json: serde_json::json!({ "kind": "internal", "message": e.to_string() }).to_string() }),
        }
    }

    /// Ends every session, tunnel and file browser.
    pub fn stop(&self) {
        self.core.stop();
    }
}

/// Android: lets the core check TLS certificates (sync, cloud drives, GitHub) with the system's
/// store: rustls-platform-verifier calls into the JVM for it and has to be handed the app's context
/// first. Called once at start by `dev.detour.android.core.Native.init(context)`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_detour_android_core_Native_init<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    context: jni::objects::JObject<'caller>,
) {
    unowned_env
        .with_env(|env| -> jni::errors::Result<()> { rustls_platform_verifier::android::init_with_env(env, context) })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
