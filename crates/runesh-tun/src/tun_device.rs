use crate::error::TunError;
use bytes::Bytes;
use std::net::Ipv4Addr;

/// Configuration for creating a TUN device.
pub struct TunConfig {
    /// Interface name (alphanumeric and hyphens only, max 15 chars)
    pub name: String,
    /// IP address to assign to the interface
    pub address: Ipv4Addr,
    /// Netmask for the interface
    pub netmask: Ipv4Addr,
    /// Maximum transmission unit
    pub mtu: u16,
}

impl TunConfig {
    /// Validate the interface name to prevent command injection.
    fn validate(&self) -> Result<(), TunError> {
        if self.name.is_empty() || self.name.len() > 15 {
            return Err(TunError::Network(
                "Interface name must be 1-15 characters".into(),
            ));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(TunError::Network(
                "Interface name must contain only alphanumeric characters and hyphens".into(),
            ));
        }
        Ok(())
    }
}

// ---- Windows implementation using Wintun ----------------------------------------

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::sync::Arc;
    use tracing::{error, info};

    pub struct TunDevice {
        name: String,
        mtu: u16,
        session: Arc<wintun::Session>,
        _adapter: Arc<wintun::Adapter>,
    }

    // Safety: Wintun session handles are thread-safe
    unsafe impl Send for TunDevice {}
    unsafe impl Sync for TunDevice {}

    impl TunDevice {
        pub fn create(config: TunConfig) -> Result<Self, TunError> {
            config.validate()?;

            // Load wintun.dll from same directory as executable, or system path
            let dll_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("wintun.dll")))
                .filter(|p| p.exists());

            let wintun = if let Some(ref path) = dll_path {
                unsafe { wintun::load_from_path(path) }
            } else {
                unsafe { wintun::load() }
            }
            .map_err(|e| {
                TunError::Network(format!(
                    "Failed to load wintun.dll: {e}. Place wintun.dll next to the executable."
                ))
            })?;

            // Create adapter with a fixed GUID so it persists across restarts
            let adapter = match wintun::Adapter::create(&wintun, &config.name, "RUNESH", None) {
                Ok(a) => a,
                Err(e) => {
                    return Err(TunError::Network(format!(
                        "Failed to create TUN adapter '{}': {e}. Run as Administrator.",
                        config.name
                    )));
                }
            };

            // Set IP address using netsh
            let ip_str = config.address.to_string();
            let mask_str = config.netmask.to_string();
            let out = std::process::Command::new("netsh")
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .args([
                    "interface",
                    "ip",
                    "set",
                    "address",
                    &format!("name={}", config.name),
                    "source=static",
                    &format!("addr={ip_str}"),
                    &format!("mask={mask_str}"),
                    "gateway=none",
                ])
                .output();

            match out {
                Ok(o) if !o.status.success() => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    // Try alternative command
                    let _ = std::process::Command::new("netsh")
                        .creation_flags(0x08000000)
                        .args([
                            "interface",
                            "ip",
                            "add",
                            "address",
                            &format!("name={}", config.name),
                            &format!("addr={ip_str}"),
                            &format!("mask={mask_str}"),
                        ])
                        .output();
                    if !stderr.is_empty() {
                        tracing::warn!(stderr = %stderr, "netsh set address warning");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "netsh command failed");
                }
                _ => {}
            }

            // Start session
            let session = adapter
                .start_session(wintun::MAX_RING_CAPACITY)
                .map_err(|e| TunError::Network(format!("Failed to start Wintun session: {e}")))?;

            info!(
                name = %config.name,
                address = %config.address,
                mtu = config.mtu,
                "TUN adapter created"
            );

            Ok(Self {
                name: config.name,
                mtu: config.mtu,
                session: Arc::new(session),
                _adapter: adapter,
            })
        }

        /// Read a packet from the TUN device (blocking).
        pub fn read_blocking(&self) -> Option<Bytes> {
            match self.session.receive_blocking() {
                Ok(packet) => Some(Bytes::copy_from_slice(packet.bytes())),
                Err(e) => {
                    error!(error = %e, "TUN read error");
                    None
                }
            }
        }

        /// Write a packet to the TUN device.
        pub fn write(&self, data: &[u8]) -> Result<(), TunError> {
            let mut packet = self
                .session
                .allocate_send_packet(data.len() as u16)
                .map_err(|e| TunError::Network(format!("TUN allocate failed: {e}")))?;
            packet.bytes_mut().copy_from_slice(data);
            self.session.send_packet(packet);
            Ok(())
        }

        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn mtu(&self) -> u16 {
            self.mtu
        }
    }

    impl Drop for TunDevice {
        fn drop(&mut self) {
            info!(name = %self.name, "closing TUN adapter");
        }
    }
}

// ---- Linux implementation using /dev/net/tun ------------------------------------

#[cfg(not(windows))]
mod platform {
    use super::*;
    use tracing::info;

    pub struct TunDevice {
        name: String,
        mtu: u16,
        fd: i32,
    }

    impl TunDevice {
        pub fn create(config: TunConfig) -> Result<Self, TunError> {
            config.validate()?;

            let iface = &config.name;

            // Delete if exists
            let _ = std::process::Command::new("ip")
                .args(["link", "delete", iface])
                .output();

            // Create
            let out = std::process::Command::new("ip")
                .args(["tuntap", "add", "dev", iface, "mode", "tun"])
                .output()
                .map_err(|e| TunError::Network(format!("ip tuntap add failed: {e}")))?;

            if !out.status.success() {
                return Err(TunError::Network(format!(
                    "Failed to create TUN: {}",
                    String::from_utf8_lossy(&out.stderr)
                )));
            }

            // Set IP
            let ip_cidr = format!("{}/{}", config.address, cidr_prefix(&config.netmask));
            let _ = std::process::Command::new("ip")
                .args(["addr", "add", &ip_cidr, "dev", iface])
                .output();

            // Set MTU and bring up
            let _ = std::process::Command::new("ip")
                .args(["link", "set", iface, "mtu", &config.mtu.to_string(), "up"])
                .output();

            // Configure kernel params for overlay networking
            let sysctl_path = format!("/proc/sys/net/ipv4/conf/{iface}");
            let _ = std::fs::write(format!("{sysctl_path}/rp_filter"), "0");
            let _ = std::fs::write(format!("{sysctl_path}/accept_local"), "1");

            // Open the TUN fd
            let fd = unsafe {
                let fd = libc::open(b"/dev/net/tun\0".as_ptr() as *const _, libc::O_RDWR);
                if fd < 0 {
                    return Err(TunError::Network("Cannot open /dev/net/tun".into()));
                }

                #[repr(C)]
                struct IfReq {
                    ifr_name: [u8; 16],
                    ifr_flags: i16,
                    _pad: [u8; 22],
                }

                let mut req = IfReq {
                    ifr_name: [0u8; 16],
                    ifr_flags: 0x0001 | 0x1000, // IFF_TUN | IFF_NO_PI
                    _pad: [0u8; 22],
                };

                let name_bytes = iface.as_bytes();
                let len = name_bytes.len().min(15);
                req.ifr_name[..len].copy_from_slice(&name_bytes[..len]);

                if libc::ioctl(fd, 0x400454CA, &req) < 0 {
                    // TUNSETIFF
                    libc::close(fd);
                    return Err(TunError::Network("TUNSETIFF ioctl failed".into()));
                }
                fd
            };

            info!(name = %iface, address = %config.address, mtu = config.mtu, "TUN adapter created");

            Ok(Self {
                name: config.name,
                mtu: config.mtu,
                fd,
            })
        }

        pub fn read_blocking(&self) -> Option<Bytes> {
            let mut buf = vec![0u8; self.mtu as usize + 64];
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n > 0 {
                buf.truncate(n as usize);
                Some(Bytes::from(buf))
            } else {
                None
            }
        }

        pub fn write(&self, data: &[u8]) -> Result<(), TunError> {
            let n = unsafe { libc::write(self.fd, data.as_ptr() as *const _, data.len()) };
            if n < 0 {
                Err(TunError::Network("TUN write failed".into()))
            } else {
                Ok(())
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn mtu(&self) -> u16 {
            self.mtu
        }
    }

    impl Drop for TunDevice {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.fd);
            }
            let _ = std::process::Command::new("ip")
                .args(["link", "delete", &self.name])
                .output();
            info!(name = %self.name, "closed TUN adapter");
        }
    }

    fn cidr_prefix(mask: &Ipv4Addr) -> u8 {
        let bits = u32::from_be_bytes(mask.octets());
        bits.count_ones() as u8
    }

    mod libc {
        extern "C" {
            pub fn open(path: *const i8, flags: i32) -> i32;
            pub fn close(fd: i32) -> i32;
            pub fn read(fd: i32, buf: *mut std::ffi::c_void, count: usize) -> isize;
            pub fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
            pub fn ioctl(fd: i32, request: u64, ...) -> i32;
        }
        pub const O_RDWR: i32 = 2;
    }
}

pub use platform::TunDevice;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_creation() {
        let _config = TunConfig {
            name: "runesh0".to_string(),
            address: Ipv4Addr::new(100, 64, 0, 1),
            netmask: Ipv4Addr::new(255, 192, 0, 0),
            mtu: 1420,
        };
    }
}
