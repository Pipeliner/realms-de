//! Private generated River protocol client bindings.
//!
//! These types are a compile-time prerequisite only. The production backend
//! must not use `wayland_client`'s stock unbounded dispatch/read paths; ADR 0021
//! requires a private bounded transport adaptation before runtime use.

#![allow(dead_code)]

use wayland_client::{self, protocol::*};

mod __interfaces {
    mod window_management {
        use wayland_client::{backend as wayland_backend, protocol::__interfaces::*};

        wayland_scanner::generate_interfaces!(
            "src/backend/river_protocols/river-window-management-v1.xml"
        );
    }

    mod layer_shell {
        use super::window_management::*;
        use wayland_client::backend as wayland_backend;

        wayland_scanner::generate_interfaces!(
            "src/backend/river_protocols/river-layer-shell-v1.xml"
        );
    }

    mod xkb_bindings {
        use super::window_management::*;
        use wayland_client::backend as wayland_backend;

        wayland_scanner::generate_interfaces!(
            "src/backend/river_protocols/river-xkb-bindings-v1.xml"
        );
    }

    mod input_management {
        use wayland_client::{backend as wayland_backend, protocol::__interfaces::*};

        wayland_scanner::generate_interfaces!(
            "src/backend/river_protocols/river-input-management-v1.xml"
        );
    }

    mod libinput_config {
        use super::input_management::*;
        use wayland_client::backend as wayland_backend;

        wayland_scanner::generate_interfaces!(
            "src/backend/river_protocols/river-libinput-config-v1.xml"
        );
    }

    pub(super) use input_management::*;
    pub(super) use layer_shell::*;
    pub(super) use libinput_config::*;
    pub(super) use window_management::*;
    pub(super) use xkb_bindings::*;
}

use self::__interfaces::*;

wayland_scanner::generate_client_code!(
    "src/backend/river_protocols/river-window-management-v1.xml"
);
wayland_scanner::generate_client_code!("src/backend/river_protocols/river-layer-shell-v1.xml");
wayland_scanner::generate_client_code!("src/backend/river_protocols/river-xkb-bindings-v1.xml");
wayland_scanner::generate_client_code!("src/backend/river_protocols/river-input-management-v1.xml");
wayland_scanner::generate_client_code!("src/backend/river_protocols/river-libinput-config-v1.xml");

#[cfg(test)]
mod tests {
    use wayland_client::Proxy;

    use super::{
        river_decoration_v1::RiverDecorationV1, river_input_device_v1::RiverInputDeviceV1,
        river_input_manager_v1::RiverInputManagerV1,
        river_layer_shell_output_v1::RiverLayerShellOutputV1,
        river_layer_shell_seat_v1::RiverLayerShellSeatV1, river_layer_shell_v1::RiverLayerShellV1,
        river_libinput_accel_config_v1::RiverLibinputAccelConfigV1,
        river_libinput_config_v1::RiverLibinputConfigV1,
        river_libinput_device_v1::RiverLibinputDeviceV1,
        river_libinput_result_v1::RiverLibinputResultV1, river_node_v1::RiverNodeV1,
        river_output_v1::RiverOutputV1, river_pointer_binding_v1::RiverPointerBindingV1,
        river_seat_v1::RiverSeatV1, river_shell_surface_v1::RiverShellSurfaceV1,
        river_window_manager_v1::RiverWindowManagerV1, river_window_v1::RiverWindowV1,
        river_xkb_binding_v1::RiverXkbBindingV1,
        river_xkb_bindings_seat_v1::RiverXkbBindingsSeatV1,
        river_xkb_bindings_v1::RiverXkbBindingsV1,
    };

    fn assert_interface<T: Proxy>(expected_name: &str, expected_version: u32) {
        let interface = T::interface();
        assert_eq!(interface.name, expected_name);
        assert_eq!(interface.version, expected_version);
    }

    #[test]
    fn river_v0_4_8_protocol_types_and_versions_are_bound() {
        assert_interface::<RiverWindowManagerV1>("river_window_manager_v1", 5);
        assert_interface::<RiverWindowV1>("river_window_v1", 5);
        assert_interface::<RiverDecorationV1>("river_decoration_v1", 5);
        assert_interface::<RiverShellSurfaceV1>("river_shell_surface_v1", 5);
        assert_interface::<RiverNodeV1>("river_node_v1", 5);
        assert_interface::<RiverOutputV1>("river_output_v1", 5);
        assert_interface::<RiverSeatV1>("river_seat_v1", 5);
        assert_interface::<RiverPointerBindingV1>("river_pointer_binding_v1", 5);

        assert_interface::<RiverLayerShellV1>("river_layer_shell_v1", 1);
        assert_interface::<RiverLayerShellOutputV1>("river_layer_shell_output_v1", 1);
        assert_interface::<RiverLayerShellSeatV1>("river_layer_shell_seat_v1", 1);

        assert_interface::<RiverXkbBindingsV1>("river_xkb_bindings_v1", 3);
        assert_interface::<RiverXkbBindingV1>("river_xkb_binding_v1", 3);
        assert_interface::<RiverXkbBindingsSeatV1>("river_xkb_bindings_seat_v1", 3);

        assert_interface::<RiverInputManagerV1>("river_input_manager_v1", 2);
        assert_interface::<RiverInputDeviceV1>("river_input_device_v1", 2);

        assert_interface::<RiverLibinputConfigV1>("river_libinput_config_v1", 2);
        assert_interface::<RiverLibinputDeviceV1>("river_libinput_device_v1", 2);
        assert_interface::<RiverLibinputAccelConfigV1>("river_libinput_accel_config_v1", 1);
        assert_interface::<RiverLibinputResultV1>("river_libinput_result_v1", 1);
    }
}
