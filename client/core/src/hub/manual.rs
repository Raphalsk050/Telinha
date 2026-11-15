//! The first connection with someone, made with codes: one side shares and sends an invite, the
//! other pastes it and sends an answer back. The two codes together are what both sides keep to
//! find each other afterwards, so a connection that opens also saves a contact.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;

use super::{Hub, HubEvent, ShareChoice};
use crate::engine::{EngineEvent, EngineSession, ShareAudio, ShareRequest, build_share_args, build_watch_args, resolve_engine, stream_ports};
use crate::envelope::normalize_code;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualRole {
    Share,
    Watch,
}

pub(super) struct Manual {
    session: Arc<EngineSession>,
    role: ManualRole,
    invite: Option<String>,
    answer: Option<String>,
    connected: bool,
    /// The name the contact gets when the connection opens.
    contact_name: String,
}

impl Hub {
    /// Starts the engine for a connection with codes. Sharing needs what to share. Must be
    /// called from inside a tokio runtime. The error is a sentence ready to show.
    pub fn start_manual(self: &Arc<Self>, role: ManualRole, choice: Option<ShareChoice>, contact_name: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.manual.as_ref().is_some_and(|manual| !manual.session.exited()) {
            return Err("já existe uma conexão com código em andamento".into());
        }
        let exe = resolve_engine().ok_or("não encontrei o motor de captura (telinha.exe)")?;
        let args = match (role, choice) {
            (ManualRole::Share, Some(choice)) => {
                let audio = match choice.audio {
                    ShareAudio::System { .. } => ShareAudio::System { exclude_pid: Some(std::process::id()) },
                    other => other,
                };
                build_share_args(&ShareRequest {
                    kind: choice.kind,
                    handle: Some(choice.handle),
                    index: None,
                    audio,
                    fps: choice.fps,
                    multi: false,
                    relay: None,
                    ports: stream_ports(),
                })?
            }
            (ManualRole::Share, None) => return Err("escolha uma tela, um programa ou uma placa de captura para compartilhar".into()),
            (ManualRole::Watch, _) => {
                let mut args = build_watch_args(false, true, "", stream_ports());
                // For the tests: the viewer runs without a window and without sound.
                if std::env::var_os("TELINHA_HEADLESS_VIEWER").is_some() {
                    args.extend(["--headless".to_owned(), "--no-audio".to_owned()]);
                }
                args
            }
        };
        let (session, events) = EngineSession::start(&exe, &args)?;
        let session = Arc::new(session);
        inner.manual = Some(Manual {
            session: Arc::clone(&session),
            role,
            invite: None,
            answer: None,
            connected: false,
            contact_name: contact_name.to_owned(),
        });
        drop(inner);
        self.pump_manual(session, events);
        Ok(())
    }

    fn pump_manual(self: &Arc<Self>, session: Arc<EngineSession>, mut events: mpsc::UnboundedReceiver<EngineEvent>) {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    EngineEvent::Event(event) => hub.manual_event(&session, event),
                    EngineEvent::Log(line) => hub.engine_log("codigo", &line),
                    EngineEvent::Exit { code, stopped } => {
                        let mut inner = hub.lock();
                        if inner.manual.as_ref().is_some_and(|manual| Arc::ptr_eq(&manual.session, &session)) {
                            inner.manual = None;
                        }
                        drop(inner);
                        hub.emit(HubEvent::ManualEnded { code, stopped });
                        return;
                    }
                }
            }
        });
    }

    fn manual_event(&self, session: &Arc<EngineSession>, event: Value) {
        let kind = event.get("event").and_then(Value::as_str).unwrap_or("");
        let mut save = None;
        {
            let mut inner = self.lock();
            let Some(manual) = inner.manual.as_mut().filter(|manual| Arc::ptr_eq(&manual.session, session)) else { return };
            if kind == "code" {
                let code = event.get("code").and_then(Value::as_str).map(normalize_code);
                if event.get("kind").and_then(Value::as_str) == Some("invite") {
                    manual.invite = code;
                } else {
                    manual.answer = code;
                }
            } else if kind == "state" && event.get("state").and_then(Value::as_str) == Some("Connected") && !manual.connected {
                manual.connected = true;
                if let (Some(invite), Some(answer)) = (&manual.invite, &manual.answer) {
                    save = Some((invite.clone(), answer.clone(), manual.contact_name.clone()));
                }
            }
        }
        if let Some((invite, answer, name)) = save {
            let (contact, created) = self.save_contact_from_codes(&invite, &answer, &name);
            self.emit(HubEvent::ContactSaved { contact_id: contact.id, name: contact.name, created });
        }
        self.emit(HubEvent::Manual(event));
    }

    pub fn manual_role(&self) -> Option<ManualRole> {
        self.lock().manual.as_ref().map(|manual| manual.role)
    }

    /// Hands the code of the other person to the engine. False when it is not a code at all.
    pub fn submit_manual_code(&self, code: &str) -> bool {
        let mut inner = self.lock();
        let Some(manual) = inner.manual.as_mut() else { return false };
        let accepted = manual.session.submit_code(code);
        if accepted {
            let code = Some(normalize_code(code));
            match manual.role {
                ManualRole::Share => manual.answer = code,
                ManualRole::Watch => manual.invite = code,
            }
        }
        accepted
    }

    /// Sends a command to the engine of the connection with codes, once it is connected.
    pub fn manual_command(&self, role: ManualRole, command: &Value) -> bool {
        self.lock().manual.as_ref().is_some_and(|manual| manual.role == role && manual.connected && manual.session.send_command(command))
    }

    pub fn stop_manual(&self) {
        if let Some(manual) = &self.lock().manual {
            manual.session.stop();
        }
    }
}
