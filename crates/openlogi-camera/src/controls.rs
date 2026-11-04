//! Platform-independent control vocabulary shared by every UVC backend
//! (IOKit on macOS, DirectShow on Windows, stubs elsewhere).

use std::collections::BTreeMap;

use thiserror::Error;

/// One adjustable camera control, mapped to a UVC selector by each backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraControl {
    Zoom,
    Pan,
    Tilt,
    Focus,
    Exposure,
    PowerLineFrequency,
    LowLightCompensation,
    Brightness,
    Contrast,
    Saturation,
    Sharpness,
    Gain,
    BacklightCompensation,
    WhiteBalance,
    Tint,
}

impl CameraControl {
    /// Every control, in the order the UI lists them (lens first, then image).
    pub const ALL: [Self; 15] = [
        Self::Zoom,
        Self::Pan,
        Self::Tilt,
        Self::Focus,
        Self::Exposure,
        Self::PowerLineFrequency,
        Self::LowLightCompensation,
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Sharpness,
        Self::Gain,
        Self::BacklightCompensation,
        Self::WhiteBalance,
        Self::Tint,
    ];

    /// Stable snake_case identifier used for config persistence and the CLI.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Zoom => "zoom",
            Self::Pan => "pan",
            Self::Tilt => "tilt",
            Self::Focus => "focus",
            Self::Exposure => "exposure",
            Self::PowerLineFrequency => "power_line_frequency",
            Self::LowLightCompensation => "low_light_compensation",
            Self::Brightness => "brightness",
            Self::Contrast => "contrast",
            Self::Saturation => "saturation",
            Self::Sharpness => "sharpness",
            Self::Gain => "gain",
            Self::BacklightCompensation => "backlight_compensation",
            Self::WhiteBalance => "white_balance",
            Self::Tint => "tint",
        }
    }

    /// The auto-mode toggle that gates this control, if the device has one.
    #[must_use]
    pub fn auto_toggle(self) -> Option<AutoToggle> {
        match self {
            Self::Focus => Some(AutoToggle::Focus),
            Self::Exposure => Some(AutoToggle::Exposure),
            Self::WhiteBalance => Some(AutoToggle::WhiteBalance),
            _ => None,
        }
    }

    /// Whether a batch that applies `autos` hands this control over to its
    /// auto mode, so the batch must not write the control's manual value.
    ///
    /// The camera rejects a manual write to a control its auto mode owns. A
    /// batch that switches the auto on *and* carries the gated value — a reset
    /// to defaults, or a profile that engages every auto mode — would fail on
    /// that value, and the caller would discard the whole batch. The toggle
    /// expresses the intent; the manual value is redundant. Switching the auto
    /// *off* is exactly when the value must be written, so that stays writable.
    #[must_use]
    pub fn handed_to_auto(self, autos: &[(AutoToggle, bool)]) -> bool {
        self.auto_toggle()
            .is_some_and(|gate| autos.contains(&(gate, true)))
    }
}

/// An auto-mode toggle paired with a manual control (focus / exposure / white
/// balance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoToggle {
    Focus,
    Exposure,
    WhiteBalance,
}

impl AutoToggle {
    /// Every toggle, matching [`CameraControl::auto_toggle`] pairs.
    pub const ALL: [Self; 3] = [Self::Focus, Self::Exposure, Self::WhiteBalance];

    /// Stable snake_case identifier used for config persistence and the CLI.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Focus => "focus_auto",
            Self::Exposure => "exposure_auto",
            Self::WhiteBalance => "white_balance_auto",
        }
    }
}

/// One auto toggle's live and default state, read from the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoState {
    pub current: bool,
    pub default: bool,
}

/// Everything the controls UI needs, read in a single device-open: each
/// supported control's range and each supported auto toggle's state.
#[derive(Debug, Clone, Default)]
pub struct CameraState {
    pub controls: Vec<(CameraControl, ControlRange)>,
    pub autos: Vec<(AutoToggle, AutoState)>,
}

/// The auto modes and control values saved for one camera, which
/// [`CameraState::reconcile`] writes back to the device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedSettings {
    pub autos: Vec<(AutoToggle, bool)>,
    pub values: Vec<(CameraControl, i32)>,
}

impl SavedSettings {
    /// Read the settings out of the config's name → value map, where an auto
    /// mode is stored as `0` (off) or non-zero (on).
    #[must_use]
    pub fn from_named(saved: &BTreeMap<String, i32>) -> Self {
        Self {
            autos: AutoToggle::ALL
                .into_iter()
                .filter_map(|toggle| saved.get(toggle.name()).map(|v| (toggle, *v != 0)))
                .collect(),
            values: CameraControl::ALL
                .into_iter()
                .filter_map(|control| saved.get(control.name()).map(|v| (control, *v)))
                .collect(),
        }
    }

    fn auto(&self, toggle: AutoToggle) -> Option<bool> {
        self.autos
            .iter()
            .find(|(t, _)| *t == toggle)
            .map(|(_, on)| *on)
    }

    fn value(&self, control: CameraControl) -> Option<i32> {
        self.values
            .iter()
            .find(|(c, _)| *c == control)
            .map(|(_, v)| *v)
    }
}

/// The writes that bring a device back to its saved settings, in the order
/// `apply_settings` takes them: auto modes first, then values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reapply {
    pub autos: Vec<(AutoToggle, bool)>,
    pub values: Vec<(CameraControl, i32)>,
}

impl Reapply {
    /// Whether the device already matches its saved settings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.autos.is_empty() && self.values.is_empty()
    }
}

impl CameraState {
    /// Settle this state to `saved` and return the writes that get the device
    /// there. The single owner of how saved settings win over a camera that
    /// came up with its own: saved auto modes win over the device's, then saved
    /// values, clamped to the device range, win for every control whose auto
    /// mode stays off. A control left to its auto mode keeps its saved value
    /// in the state but is not written, since the camera rejects a manual
    /// write there.
    ///
    /// Afterwards each `current` is what the device holds once the returned
    /// writes land.
    pub fn reconcile(&mut self, saved: &SavedSettings) -> Reapply {
        let mut reapply = Reapply::default();
        for (toggle, state) in &mut self.autos {
            let on = saved.auto(*toggle).unwrap_or(state.current);
            if on != state.current {
                reapply.autos.push((*toggle, on));
            }
            state.current = on;
        }
        for (control, range) in &mut self.controls {
            let saved_value = saved.value(*control);
            let target = saved_value
                .unwrap_or(range.current)
                .max(range.min)
                .min(range.max);
            let auto_on = control.auto_toggle().is_some_and(|toggle| {
                self.autos
                    .iter()
                    .any(|(t, state)| *t == toggle && state.current)
            });
            if saved_value.is_some() && target != range.current && !auto_on {
                reapply.values.push((*control, target));
            }
            range.current = target;
        }
        reapply
    }
}

/// The device's reported range and current value for a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRange {
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub current: i32,
    /// Bit `n` is set when discrete value `n` is supported. `None` means every
    /// value in the range is available.
    pub value_mask: Option<u32>,
}

impl ControlRange {
    /// Whether the device reports `value` as supported.
    #[must_use]
    pub fn supports(self, value: i32) -> bool {
        if !(self.min..=self.max).contains(&value) {
            return false;
        }
        self.value_mask.is_none_or(|mask| {
            u32::try_from(value)
                .ok()
                .filter(|value| *value < u32::BITS)
                .is_some_and(|value| mask & (1 << value) != 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        AutoState, AutoToggle, CameraControl, CameraState, ControlRange, Reapply, SavedSettings,
    };

    fn range(current: i32) -> ControlRange {
        ControlRange {
            min: 0,
            max: 255,
            default: 128,
            current,
            value_mask: None,
        }
    }

    #[test]
    fn saved_settings_read_autos_as_flags_and_skip_unknown_names() {
        let named = BTreeMap::from([
            ("brightness".to_string(), 140),
            ("focus_auto".to_string(), 0),
            ("not_a_control".to_string(), 7),
        ]);
        let saved = SavedSettings::from_named(&named);
        assert_eq!(saved.values, [(CameraControl::Brightness, 140)]);
        assert_eq!(saved.autos, [(AutoToggle::Focus, false)]);
    }

    #[test]
    fn reconcile_writes_only_the_values_the_device_lost() {
        let mut state = CameraState {
            controls: vec![
                (CameraControl::Brightness, range(128)),
                (CameraControl::Contrast, range(150)),
            ],
            autos: Vec::new(),
        };
        let saved = SavedSettings {
            autos: Vec::new(),
            values: vec![
                (CameraControl::Brightness, 140),
                (CameraControl::Contrast, 150),
                // Not on this device.
                (CameraControl::Saturation, 155),
            ],
        };

        let reapply = state.reconcile(&saved);

        // Contrast already matches; saturation isn't reported by the device.
        assert_eq!(
            reapply,
            Reapply {
                autos: Vec::new(),
                values: vec![(CameraControl::Brightness, 140)],
            }
        );
        assert_eq!(state.controls[0].1.current, 140);
    }

    #[test]
    fn reconcile_clamps_a_saved_value_to_the_device_range() {
        let mut state = CameraState {
            controls: vec![(CameraControl::Brightness, range(255))],
            autos: Vec::new(),
        };
        let saved = SavedSettings {
            autos: Vec::new(),
            values: vec![(CameraControl::Brightness, 900)],
        };

        // Clamped to 255, which the device already holds: nothing to write.
        assert!(state.reconcile(&saved).is_empty());
        assert_eq!(state.controls[0].1.current, 255);
    }

    #[test]
    fn a_saved_auto_mode_wins_and_its_manual_value_is_not_written() {
        let toggle = AutoToggle::Focus;
        let mut state = CameraState {
            controls: vec![(CameraControl::Focus, range(10))],
            autos: vec![(
                toggle,
                AutoState {
                    current: false,
                    default: true,
                },
            )],
        };
        let saved = SavedSettings {
            autos: vec![(toggle, true)],
            values: vec![(CameraControl::Focus, 0)],
        };

        let reapply = state.reconcile(&saved);

        assert_eq!(reapply.autos, [(toggle, true)]);
        assert!(reapply.values.is_empty(), "auto focus owns the value");
        // The saved value still shows once the user switches auto off.
        assert_eq!(state.controls[0].1.current, 0);
        assert!(state.autos[0].1.current);
    }

    #[test]
    fn discrete_range_rejects_missing_values() {
        let range = ControlRange {
            min: 0,
            max: 3,
            default: 1,
            current: 1,
            value_mask: Some((1 << 1) | (1 << 3)),
        };

        assert!(range.supports(1));
        assert!(!range.supports(2));
        assert!(range.supports(3));
    }

    #[test]
    fn a_control_handed_to_auto_is_skipped() {
        let autos = [(AutoToggle::Exposure, true)];
        // Exposure is gated by the toggle being switched on...
        assert!(CameraControl::Exposure.handed_to_auto(&autos));
        // ...while ungated controls, and controls gated by a *different*
        // toggle, still apply.
        assert!(!CameraControl::Zoom.handed_to_auto(&autos));
        assert!(!CameraControl::Focus.handed_to_auto(&autos));
    }

    #[test]
    fn a_control_taken_off_auto_still_applies() {
        // Switching auto *off* is exactly when the manual value must be written.
        let autos = [(AutoToggle::Focus, false)];
        assert!(!CameraControl::Focus.handed_to_auto(&autos));
    }

    #[test]
    fn an_unmentioned_toggle_leaves_its_control_writable() {
        assert!(!CameraControl::WhiteBalance.handed_to_auto(&[]));
    }

    #[test]
    fn a_reset_that_engages_every_auto_writes_only_the_ungated_values() {
        // The controls panel's reset (and the built-in profiles) switch every
        // auto mode on in the same batch as a value for every control. Only
        // the three auto-owned values drop out; everything else still lands.
        let autos = AutoToggle::ALL.map(|toggle| (toggle, true));
        for control in CameraControl::ALL {
            let gated = matches!(
                control,
                CameraControl::Focus | CameraControl::Exposure | CameraControl::WhiteBalance
            );
            assert_eq!(control.handed_to_auto(&autos), gated, "{control:?}");
        }
    }
}

/// Why a UVC control operation failed.
#[derive(Debug, Clone, Error)]
pub enum ControlError {
    /// No matching camera device (or it exposes no controllable unit).
    #[error("no matching UVC device")]
    NotFound,
    /// The selected camera can't be uniquely identified: its unique id didn't
    /// resolve to a USB location and more than one Logitech camera is attached,
    /// so a write could hit the wrong device. Fails closed instead of guessing.
    #[error("camera could not be uniquely identified")]
    Ambiguous,
    /// The camera rejected or didn't support the control — or the platform
    /// has no UVC control backend at all.
    #[error("camera does not support that control")]
    Unsupported,
    /// A platform API call failed (open, bind, or the control transfer).
    #[error("platform error: {0}")]
    Io(String),
}
