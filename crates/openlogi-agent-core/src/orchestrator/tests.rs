//! Orchestrator tests: the shared fixtures, and one module per area.

use super::{
    AgentDevice, InventoryHealth, Orchestrator, VOLATILE_REAPPLY_CONFIRM_RETRIES,
    any_device_needs_capture_rearm, build_devices, configured_wheel_mode, host_switch_links,
    pick_current, plan_reapply, reapply_targets, stable_id,
};
use crate::hardware::WheelModeChange;
use openlogi_core::app::ForegroundApp;
use openlogi_core::binding::{Action, Binding, ButtonId};
use openlogi_core::config::{
    Config, DeviceConfig, LightSettings, LinkConfig, ScrollResolution, VerticalScrollSensitivity,
};
use openlogi_core::device::{
    Capabilities, DeviceInventory, DeviceKind, DeviceModelInfo, DeviceTransports,
    LightCapabilities, PairedDevice, RawDeviceAddress, ReceiverInfo, StandaloneDevice,
};
use openlogi_core::device_order::{DeviceIdentity, DeviceStableId};
use openlogi_core::hid::Dpi;
use openlogi_hid::{DIRECT_DEVICE_INDEX, DeviceRoute};
use std::sync::Arc;

use crate::observable::ObservableState;

mod camera;
mod capture_plans;
mod device_list;
mod device_settings;
mod publication;
mod reapply;

/// An orchestrator wired to a state cell nobody subscribes to. The publishing
/// paths still run, so a mutator that stops republishing shows up here rather
/// than only in the running agent.
fn orchestrator(config: Config) -> Orchestrator {
    Orchestrator::new(config, Arc::new(ObservableState::new("test".to_string())))
}

fn dev(key: &str, slot: u8, online: bool) -> AgentDevice {
    AgentDevice {
        config_key: key.to_string(),
        model_key: key.to_string(),
        route: Some(DeviceRoute::Bolt {
            receiver_uid: "AA00".to_string(),
            slot,
        }),
        slot,
        serial: None,
        unit_id: [0; 4],
        capabilities: None,
        kind: openlogi_core::device::DeviceKind::Mouse,
        light_capabilities: None,
        online,
    }
}

fn direct_inventory(serial_number: Option<&str>, unit_id: [u8; 4]) -> DeviceInventory {
    DeviceInventory {
        receiver: ReceiverInfo {
            name: "MX Master 3S".to_string(),
            vendor_id: 0x046d,
            product_id: 0xb023,
            unique_id: None,
        },
        paired: vec![PairedDevice {
            slot: DIRECT_DEVICE_INDEX,
            codename: Some("MX Master 3S".to_string()),
            wpid: None,
            kind: DeviceKind::Mouse,
            online: true,
            battery: None,
            model_info: Some(DeviceModelInfo {
                entity_count: 1,
                serial_number: serial_number.map(str::to_string),
                unit_id,
                transports: DeviceTransports::default(),
                model_ids: [0xb034, 0, 0],
                extended_model_id: 2,
            }),
            capabilities: Some(Capabilities::presumed_from_kind(DeviceKind::Mouse)),
        }],
    }
}

fn lit(enabled: bool) -> openlogi_core::config::Lighting {
    openlogi_core::config::Lighting {
        enabled,
        color: "0000ff".parse().expect("valid hex"),
        brightness: 100,
    }
}

#[test]
fn lighting_reassert_targets_online_devices_with_lighting_on() {
    let mut config = Config::default();
    config.set_lighting("kbd", lit(true));
    let mut orch = orchestrator(config);
    orch.devices = vec![dev("kbd", 1, true)];

    let targets = orch.lighting_reassert_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].1.color, "0000ff".parse().expect("valid hex"));
}

#[test]
fn lighting_reassert_skips_off_offline_and_disabled_devices() {
    // Re-asserting "off" would paint black every tick over the firmware's own
    // effect; offline and disabled devices get no writes of any kind.
    let mut config = Config::default();
    config.set_lighting("off", lit(false));
    config.set_lighting("offline", lit(true));
    config.set_lighting("disabled", lit(true));
    config.set_device_enabled("disabled", false);
    let mut orch = orchestrator(config);
    orch.devices = vec![
        dev("off", 1, true),
        dev("offline", 2, false),
        dev("disabled", 3, true),
    ];

    assert!(orch.lighting_reassert_targets().is_empty());
}
