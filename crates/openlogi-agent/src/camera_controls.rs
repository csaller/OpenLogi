//! Re-assert saved webcam controls when a capture session starts.
//!
//! Webcams such as the C922 come up without their UVC image settings when an
//! app starts using them, and only the GUI's camera page ever wrote them back —
//! so they stayed lost until that page was reopened. The agent sees every usage
//! transition from the camera watcher, so it re-applies the saved values then.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use openlogi_camera::SavedSettings;
use openlogi_core::config::Config;
use tracing::{debug, info, warn};

/// Let the capturing app finish setting up its session — the step that resets
/// the controls — before writing the saved ones back.
const SETTLE: Duration = Duration::from_secs(2);

/// Reading or writing controls seizes the USB device, which briefly detaches
/// the video driver; the watcher reports that as usage stopping and starting
/// again. Starts this soon after our own re-apply are that echo, not a session.
const ECHO_COOLDOWN: Duration = Duration::from_secs(10);

/// Decides which usage transitions earn a re-apply.
#[derive(Debug, Default)]
struct ReapplyGate {
    last_run: Option<Instant>,
}

impl ReapplyGate {
    /// Whether a transition to `active` at `now` should re-apply, recording the
    /// run when it does.
    fn admit(&mut self, active: bool, now: Instant) -> bool {
        let echo = self
            .last_run
            .is_some_and(|last| now.saturating_duration_since(last) < ECHO_COOLDOWN);
        if !active || echo {
            return false;
        }
        self.last_run = Some(now);
        true
    }
}

static GATE: Mutex<ReapplyGate> = Mutex::new(ReapplyGate { last_run: None });

/// React to a camera usage transition: once a session starts, re-apply every
/// connected webcam's saved controls.
pub(crate) fn on_usage_change(active: bool) {
    let admitted = GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .admit(active, Instant::now());
    if !admitted {
        return;
    }
    tokio::spawn(async {
        tokio::time::sleep(SETTLE).await;
        if let Err(error) = tokio::task::spawn_blocking(reapply_all).await {
            warn!(%error, "camera control reapply task failed");
        }
    });
}

/// Blocking body: read the config fresh from disk (the GUI saves camera
/// settings without notifying the agent), then push each camera's differences.
fn reapply_all() {
    let config = match Config::load_or_default() {
        Ok(config) => config,
        Err(error) => {
            warn!(%error, "camera control reapply: config unreadable");
            return;
        }
    };
    for camera in openlogi_camera::enumerate_cameras() {
        let key = camera.config_key();
        if !config.device_enabled(&key) {
            continue;
        }
        let Some(saved) = config.camera_controls(&key) else {
            continue;
        };
        let mut state = match openlogi_camera::read_camera_state(&camera.unique_id) {
            Ok(state) => state,
            Err(error) => {
                debug!(%error, key, "camera state read failed; skipping reapply");
                continue;
            }
        };
        let reapply = state.reconcile(&SavedSettings::from_named(&saved.0));
        if reapply.is_empty() {
            continue;
        }
        match openlogi_camera::apply_settings(&camera.unique_id, &reapply.autos, &reapply.values) {
            Ok(()) => info!(
                key,
                writes = reapply.autos.len() + reapply.values.len(),
                "re-applied saved camera controls"
            ),
            Err(error) => warn!(%error, key, "re-applying saved camera controls failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_runs_on_session_start_and_ignores_its_own_echo() {
        let mut gate = ReapplyGate::default();
        let t0 = Instant::now();
        assert!(!gate.admit(false, t0), "a session ending needs no re-apply");
        assert!(gate.admit(true, t0));
        // The seize makes usage blink off and on right after the write.
        assert!(!gate.admit(true, t0 + Duration::from_secs(3)));
        assert!(gate.admit(true, t0 + ECHO_COOLDOWN));
    }
}
