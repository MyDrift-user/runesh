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
    use sha2::{Digest, Sha256};
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::sync::Arc;
    #[allow(unused_imports)]
    use tracing::{error, info, warn};

    /// SHA-256 of the known-good Wintun DLL (hex, 64 characters). Supplied
    /// at build time via the `RUNESH_WINTUN_SHA256_HEX` environment variable
    /// so the consumer pins the exact byte-for-byte DLL they verified.
    ///
    /// Example (PowerShell):
    /// ```text
    /// $env:RUNESH_WINTUN_SHA256_HEX = "07c256185d6ee3..."; cargo build
    /// ```
    ///
    /// If this is unset at build time the default-feature build refuses to
    /// load wintun.dll at runtime. The `unpinned-wintun` feature is the
    /// explicit dev escape hatch.
    const PINNED_WINTUN_SHA256_HEX: Option<&str> = option_env!("RUNESH_WINTUN_SHA256_HEX");

    fn pinned_wintun_sha256() -> Result<Option<[u8; 32]>, TunError> {
        match PINNED_WINTUN_SHA256_HEX {
            None => Ok(None),
            Some(hex) => {
                let hex = hex.trim();
                if hex.len() != 64 {
                    return Err(TunError::Network(format!(
                        "RUNESH_WINTUN_SHA256_HEX must be 64 hex chars, got {}",
                        hex.len()
                    )));
                }
                let mut out = [0u8; 32];
                for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
                    let s = std::str::from_utf8(chunk).map_err(|e| {
                        TunError::Network(format!("invalid RUNESH_WINTUN_SHA256_HEX: {e}"))
                    })?;
                    out[i] = u8::from_str_radix(s, 16).map_err(|e| {
                        TunError::Network(format!("invalid RUNESH_WINTUN_SHA256_HEX: {e}"))
                    })?;
                }
                Ok(Some(out))
            }
        }
    }

    pub struct TunDevice {
        name: String,
        mtu: u16,
        session: Arc<wintun::Session>,
        _adapter: Arc<wintun::Adapter>,
    }

    // Safety: Wintun session handles are thread-safe
    unsafe impl Send for TunDevice {}
    unsafe impl Sync for TunDevice {}

    fn hash_file(path: &Path) -> Result<[u8; 32], TunError> {
        let bytes = std::fs::read(path)
            .map_err(|e| TunError::Network(format!("cannot read wintun.dll for pin check: {e}")))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let out = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
        Ok(arr)
    }

    fn check_wintun_pin(path: &Path) -> Result<(), TunError> {
        let pin = pinned_wintun_sha256()?;
        let actual = hash_file(path)?;
        if let Some(pin) = pin {
            if actual == pin {
                return Ok(());
            }
            #[cfg(feature = "unpinned-wintun")]
            {
                warn!(
                    path = %path.display(),
                    actual = %hex_encode(&actual),
                    "wintun.dll hash does not match RUNESH_WINTUN_SHA256_HEX; \
                     continuing because feature `unpinned-wintun` is enabled. \
                     Never enable this feature on a shipped binary."
                );
                return Ok(());
            }
            #[cfg(not(feature = "unpinned-wintun"))]
            {
                return Err(TunError::Network(format!(
                    "wintun.dll at {} does not match the pinned SHA-256 \
                     (actual {}). Refusing to load. Rebuild with the correct \
                     RUNESH_WINTUN_SHA256_HEX, or enable feature \
                     `unpinned-wintun` for dev only.",
                    path.display(),
                    hex_encode(&actual),
                )));
            }
        }

        // No pin supplied at build time.
        #[cfg(feature = "unpinned-wintun")]
        {
            warn!(
                path = %path.display(),
                actual = %hex_encode(&actual),
                "RUNESH_WINTUN_SHA256_HEX was not set at build time and feature \
                 `unpinned-wintun` is enabled; loading wintun.dll without \
                 verification. Never ship with this configuration."
            );
            return Ok(());
        }
        #[cfg(not(feature = "unpinned-wintun"))]
        {
            Err(TunError::Network(format!(
                "RUNESH_WINTUN_SHA256_HEX was not set at build time and feature \
                 `unpinned-wintun` is not enabled. Refusing to load wintun.dll \
                 at {}. Rebuild with the env var set to the verified DLL hash.",
                path.display(),
            )))
        }
    }

    fn hex_encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    impl TunDevice {
        pub fn create(config: TunConfig) -> Result<Self, TunError> {
            config.validate()?;

            // Load wintun.dll from same directory as executable, or system path
            let dll_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("wintun.dll")))
                .filter(|p| p.exists());

            if let Some(path) = &dll_path {
                check_wintun_pin(path)?;
            } else {
                #[cfg(not(feature = "unpinned-wintun"))]
                {
                    return Err(TunError::Network(
                        "wintun.dll not found next to the executable; refusing to load \
                         an unknown system copy. Bundle the pinned DLL or enable the \
                         `unpinned-wintun` feature."
                            .into(),
                    ));
                }
                #[cfg(feature = "unpinned-wintun")]
                {
                    warn!(
                        "wintun.dll not found next to the executable; falling back to \
                         the system copy because `unpinned-wintun` is enabled"
                    );
                }
            }

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
    use std::ffi::CString;
    use tracing::{info, warn};

    /// Architecture-specific TUNSETIFF ioctl request number.
    ///
    /// On x86/x86_64 TUNSETIFF = `_IOW('T', 202, int)` = 0x400454CA.
    /// On arm/aarch64/mips/riscv64 the direction bits swap so the value is
    /// 0x800454CA. Using the wrong constant silently fails or corrupts
    /// kernel state, so we hard-pin per target_arch.
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    pub const TUNSETIFF: libc::c_ulong = 0x400454CA;
    #[cfg(any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
    ))]
    pub const TUNSETIFF: libc::c_ulong = 0x800454CA;

    /// IFF_TUN | IFF_NO_PI
    const IFF_FLAGS: i16 = 0x0001 | 0x1000;

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
            let path = CString::new("/dev/net/tun")
                .map_err(|e| TunError::Network(format!("bad tun path: {e}")))?;
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
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
                ifr_flags: IFF_FLAGS,
                _pad: [0u8; 22],
            };

            let name_bytes = iface.as_bytes();
            let len = name_bytes.len().min(15);
            req.ifr_name[..len].copy_from_slice(&name_bytes[..len]);

            let ret = unsafe { libc::ioctl(fd, TUNSETIFF, &mut req as *mut _) };
            if ret < 0 {
                unsafe { libc::close(fd) };
                return Err(TunError::Network("TUNSETIFF ioctl failed".into()));
            }

            info!(name = %iface, address = %config.address, mtu = config.mtu, "TUN adapter created");

            Ok(Self {
                name: config.name,
                mtu: config.mtu,
                fd,
            })
        }

        /// Read one packet from the TUN device (blocking).
        ///
        /// Allocates `mtu + 64 + 1` bytes, where the extra guard byte lets
        /// us detect kernel truncation: a read that fills the buffer exactly
        /// to `mtu + 64` almost certainly means the next packet was larger
        /// than advertised and was silently clipped. We discard such reads.
        pub fn read_blocking(&self) -> Option<Bytes> {
            let advertised = self.mtu as usize + 64;
            let mut buf = vec![0u8; advertised + 1];
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                return None;
            }
            let n = n as usize;
            if n > advertised {
                warn!(
                    mtu = self.mtu,
                    got = n,
                    "TUN packet exceeded mtu+64; discarding to avoid truncated packet"
                );
                return None;
            }
            buf.truncate(n);
            Some(Bytes::from(buf))
        }

        pub fn write(&self, data: &[u8]) -> Result<(), TunError> {
            let n =
                unsafe { libc::write(self.fd, data.as_ptr() as *const libc::c_void, data.len()) };
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

    #[test]
    fn config_validate_accepts_typical_name() {
        let cfg = TunConfig {
            name: "wg0".into(),
            address: Ipv4Addr::new(10, 0, 0, 1),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mtu: 1420,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_validate_rejects_shell_metachars() {
        let cfg = TunConfig {
            name: "wg0;rm -rf /".into(),
            address: Ipv4Addr::new(10, 0, 0, 1),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mtu: 1420,
        };
        assert!(cfg.validate().is_err());
    }

    #[cfg(all(not(windows), any(target_arch = "x86_64", target_arch = "x86")))]
    #[test]
    fn tunsetiff_constant_matches_x86() {
        assert_eq!(super::platform::TUNSETIFF, 0x400454CA);
    }

    #[cfg(all(
        not(windows),
        any(
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv64",
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
        )
    ))]
    #[test]
    fn tunsetiff_constant_matches_arm_family() {
        assert_eq!(super::platform::TUNSETIFF, 0x800454CA);
    }
}
