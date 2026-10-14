//! The host's sound in a session (ADR 0044): off until the viewer turns it on.
//!
//! The host records and encodes with [`audio::Capture`] and sends each packet on the
//! session's audio channel; the viewer plays them with [`audio::Player`]. Both report
//! "not available" on Linux.

use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "macos", windows))]
use bytes::BytesMut;
#[cfg(any(target_os = "macos", windows))]
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use transport::{DataChannel, DataChannelEvent};

/// The audio channel, once the viewer's offer has opened it (never with peers before 1.3.0).
type Slot = Arc<Mutex<Option<Arc<dyn DataChannel>>>>;

fn fill_slot(channel: oneshot::Receiver<Arc<dyn DataChannel>>) -> Slot {
    let slot: Slot = Arc::default();
    let filled = slot.clone();
    tokio::spawn(async move {
        if let Ok(channel) = channel.await {
            *filled.lock().unwrap() = Some(channel);
        }
    });
    slot
}

/// Host side.
pub struct HostSound {
    channel: Slot,
    #[cfg(any(target_os = "macos", windows))]
    capture: Option<audio::Capture>,
}

impl HostSound {
    pub fn new(channel: oneshot::Receiver<Arc<dyn DataChannel>>) -> Self {
        Self {
            channel: fill_slot(channel),
            #[cfg(any(target_os = "macos", windows))]
            capture: None,
        }
    }

    /// Starts or stops sending the sound. `Err` (for the viewer) when it cannot start.
    pub async fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if self.channel.lock().unwrap().is_none() && enabled {
            return Err("il partner non supporta l'audio (aggiornalo alla 1.3)".into());
        }
        #[cfg(any(target_os = "macos", windows))]
        {
            if !enabled {
                if self.capture.take().is_some() {
                    tracing::info!("sound off");
                }
                return Ok(());
            }
            if self.capture.is_some() {
                return Ok(());
            }
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
            let channel = self.channel.clone();
            // Ends when the capture is dropped (its sender goes with it).
            tokio::spawn(async move {
                while let Some(packet) = rx.recv().await {
                    let channel = channel.lock().unwrap().clone();
                    if let Some(channel) = channel {
                        let _ = channel.send(BytesMut::from(&packet[..])).await;
                    }
                }
            });
            // Opening the device may take a moment: keep it off the runtime.
            let started = tokio::task::spawn_blocking(move || {
                audio::Capture::start(move |packet| {
                    let _ = tx.send(packet);
                })
            })
            .await
            .map_err(|e| e.to_string())?;
            match started {
                Ok(capture) => {
                    tracing::info!("sound on");
                    self.capture = Some(capture);
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("cannot capture the sound: {e:#}");
                    Err(format!("audio non disponibile su questo computer: {e:#}"))
                }
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            if enabled {
                return Err("audio non disponibile su Linux".into());
            }
            Ok(())
        }
    }
}

/// Viewer side.
pub struct ViewerSound {
    #[cfg(any(target_os = "macos", windows))]
    player: Arc<Mutex<Option<audio::Player>>>,
    task: tokio::task::JoinHandle<()>,
}

impl ViewerSound {
    pub fn new(channel: oneshot::Receiver<Arc<dyn DataChannel>>) -> Self {
        #[cfg(any(target_os = "macos", windows))]
        let player: Arc<Mutex<Option<audio::Player>>> = Arc::default();
        #[cfg(any(target_os = "macos", windows))]
        let playing = player.clone();
        let task = tokio::spawn(async move {
            let Ok(channel) = channel.await else {
                return;
            };
            while let Some(event) = channel.poll().await {
                match event {
                    DataChannelEvent::OnMessage(message) => {
                        #[cfg(any(target_os = "macos", windows))]
                        if let Some(player) = playing.lock().unwrap().as_ref() {
                            player.push(&message.data);
                        }
                        #[cfg(not(any(target_os = "macos", windows)))]
                        let _ = message;
                    }
                    DataChannelEvent::OnClose => break,
                    _ => {}
                }
            }
        });
        Self {
            #[cfg(any(target_os = "macos", windows))]
            player,
            task,
        }
    }

    /// Opens or closes the speakers. `Err` when this machine cannot play the sound.
    pub async fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            if !enabled {
                self.player.lock().unwrap().take();
                return Ok(());
            }
            if self.player.lock().unwrap().is_some() {
                return Ok(());
            }
            let started = tokio::task::spawn_blocking(audio::Player::start)
                .await
                .map_err(|e| e.to_string())?;
            match started {
                Ok(player) => {
                    *self.player.lock().unwrap() = Some(player);
                    Ok(())
                }
                Err(e) => Err(format!("impossibile riprodurre l'audio: {e:#}")),
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            if enabled {
                return Err("audio non disponibile su Linux".into());
            }
            Ok(())
        }
    }
}

impl Drop for ViewerSound {
    fn drop(&mut self) {
        self.task.abort();
    }
}
