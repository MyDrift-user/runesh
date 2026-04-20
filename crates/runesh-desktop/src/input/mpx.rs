//! X11 Multi-Pointer Extension (MPX) — true native multi-cursor on Linux.
//!
//! X11 is the only mainstream display server that supports multiple independent
//! system cursors. Each "master pointer" device gets its own visible cursor,
//! focus window, and button state.
//!
//! This module creates virtual master pointer devices for remote users,
//! giving them real OS-level cursors that are rendered by the X server itself.

#[cfg(target_os = "linux")]
mod mpx_impl {
    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::protocol::xinput::{
        self, ConnectionExt as XInputExt, HierarchyChange, HierarchyChangeData,
        HierarchyChangeDataAddMaster, HierarchyChangeDataRemoveMaster,
    };
    use x11rb::protocol::xproto::*;
    use x11rb::protocol::xtest::ConnectionExt as XtestExt;

    use crate::error::DesktopError;
    use crate::protocol::MouseButton;

    /// An MPX virtual cursor — a real X11 master pointer device.
    pub struct MpxCursor {
        conn: x11rb::rust_connection::RustConnection,
        /// The master pointer device ID.
        pointer_id: u16,
        /// The paired master keyboard device ID.
        keyboard_id: u16,
        /// Display name for this cursor.
        name: String,
        /// Root window for input injection.
        root: Window,
    }

    impl MpxCursor {
        /// Create a new MPX virtual cursor.
        ///
        /// This creates a new master pointer + keyboard pair in the X server.
        /// A new visible cursor appears on screen immediately.
        pub fn create(name: &str) -> Result<Self, DesktopError> {
            let (conn, screen_num) = x11rb::connect(None)
                .map_err(|e| DesktopError::Input(format!("X11 connect failed: {e}")))?;

            // Verify XInput2 extension is available
            conn.extension_information(xinput::X11_EXTENSION_NAME)
                .map_err(|e| DesktopError::Input(format!("XInput query failed: {e}")))?
                .ok_or_else(|| DesktopError::Input("XInput2 extension not available".into()))?;

            let screen = &conn.setup().roots[screen_num];
            let root = screen.root;

            // Create a new master device pair via XIChangeHierarchy
            let add_master = HierarchyChangeDataAddMaster {
                send_core: true, // allow this device to generate core events
                enable: true,
                name: name.as_bytes().to_vec(),
            };
            let change = HierarchyChange {
                len: 0, // serialized automatically
                data: HierarchyChangeData::AddMaster(add_master),
            };
            conn.xinput_xi_change_hierarchy(&[change]).map_err(|e| {
                DesktopError::Input(format!("XIChangeHierarchy AddMaster failed: {e}"))
            })?;

            conn.flush()
                .map_err(|e| DesktopError::Input(format!("X11 flush failed: {e}")))?;

            // Find the newly created device IDs by listing all devices
            let (pointer_id, keyboard_id) = find_device_by_name(&conn, name)?;

            tracing::info!(
                name = %name,
                pointer_id,
                keyboard_id,
                "MPX: Created virtual cursor"
            );

            Ok(Self {
                conn,
                pointer_id,
                keyboard_id,
                name: name.to_string(),
                root,
            })
        }

        /// Move this cursor to absolute screen coordinates.
        pub fn move_to(&self, x: i32, y: i32) -> Result<(), DesktopError> {
            // XIWarpPointer for the specific device
            self.conn
                .xinput_xi_warp_pointer(
                    x11rb::NONE,  // src_window
                    self.root,    // dst_window
                    0.into(),     // src_x (fixed-point)
                    0.into(),     // src_y
                    0,            // src_width
                    0,            // src_height
                    to_fp1616(x), // dst_x (fixed-point 16.16)
                    to_fp1616(y), // dst_y
                    xinput::DeviceId::from(self.pointer_id),
                )
                .map_err(|e| DesktopError::Input(format!("XIWarpPointer failed: {e}")))?;

            self.conn
                .flush()
                .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

            Ok(())
        }

        /// Simulate a button press/release on this cursor.
        pub fn button(
            &self,
            button: MouseButton,
            pressed: bool,
            x: i32,
            y: i32,
        ) -> Result<(), DesktopError> {
            // Move to position first
            self.move_to(x, y)?;

            let x11_button: u8 = match button {
                MouseButton::Left => 1,
                MouseButton::Middle => 2,
                MouseButton::Right => 3,
                MouseButton::Back => 8,
                MouseButton::Forward => 9,
            };

            let event_type: u8 = if pressed { 4 } else { 5 }; // ButtonPress / ButtonRelease

            // Use XTest with device specification
            self.conn
                .xtest_fake_input(
                    event_type,
                    x11_button,
                    0,
                    self.root,
                    0,
                    0,
                    self.pointer_id as u8,
                )
                .map_err(|e| DesktopError::Input(format!("FakeInput button failed: {e}")))?;

            self.conn
                .flush()
                .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

            Ok(())
        }

        /// Get the device name.
        pub fn name(&self) -> &str {
            &self.name
        }

        /// Get the pointer device ID.
        pub fn pointer_id(&self) -> u16 {
            self.pointer_id
        }

        /// Destroy this virtual cursor.
        fn destroy(&self) {
            // Remove the master device via XIChangeHierarchy
            let remove = HierarchyChangeDataRemoveMaster {
                deviceid: self.pointer_id,
                return_mode: xinput::ChangeMode::FLOAT,
                return_pointer: 0,
                return_keyboard: 0,
            };
            let change = HierarchyChange {
                len: 0,
                data: HierarchyChangeData::RemoveMaster(remove),
            };
            let _ = self.conn.xinput_xi_change_hierarchy(&[change]);
            let _ = self.conn.flush();
            tracing::info!(name = %self.name, "MPX: Destroyed virtual cursor");
        }
    }

    impl Drop for MpxCursor {
        fn drop(&mut self) {
            self.destroy();
        }
    }

    /// Convert an integer to X11 fixed-point 16.16 format.
    fn to_fp1616(value: i32) -> xinput::Fp1616 {
        xinput::Fp1616::from(value << 16)
    }

    /// Find a master device by name prefix after creation.
    fn find_device_by_name(
        conn: &x11rb::rust_connection::RustConnection,
        name: &str,
    ) -> Result<(u16, u16), DesktopError> {
        let devices = conn
            .xinput_xi_query_device(xinput::DeviceId::from(xinput::Device::ALL))
            .map_err(|e| DesktopError::Input(format!("XIQueryDevice failed: {e}")))?
            .reply()
            .map_err(|e| DesktopError::Input(format!("XIQueryDevice reply failed: {e}")))?;

        let mut pointer_id = None;
        let mut keyboard_id = None;

        for info in &devices.infos {
            let dev_name = String::from_utf8_lossy(&info.name);

            if dev_name.starts_with(name) {
                match info.type_ {
                    xinput::DeviceType::MASTER_POINTER => {
                        pointer_id = Some(info.deviceid);
                    }
                    xinput::DeviceType::MASTER_KEYBOARD => {
                        keyboard_id = Some(info.deviceid);
                    }
                    _ => {}
                }
            }
        }

        match (pointer_id, keyboard_id) {
            (Some(p), Some(k)) => Ok((p, k)),
            _ => Err(DesktopError::Input(format!(
                "Could not find MPX device '{name}' after creation"
            ))),
        }
    }

    /// Check if MPX (XInput2) is available on this system.
    pub fn is_mpx_available() -> bool {
        let Ok((conn, _)) = x11rb::connect(None) else {
            return false;
        };

        conn.extension_information(xinput::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some()
    }
}

#[cfg(target_os = "linux")]
pub use mpx_impl::{MpxCursor, is_mpx_available};
