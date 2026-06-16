//! TUN device management for WolfNet
//!
//! Creates and manages a virtual network interface using the Linux TUN driver.

use std::os::unix::io::RawFd;
use tracing::warn;

// TUNSETIFF = _IOW('T', 202, c_int)
// PowerPC encodes ioctl direction bits differently from x86/ARM:
//   x86/ARM: _IOC_WRITE = 1  → TUNSETIFF = 0x400454ca
//   PowerPC: _IOC_WRITE = 4  → TUNSETIFF = 0x800454ca
// Compute at compile time so every architecture gets the correct value.
#[cfg(any(target_arch = "powerpc", target_arch = "powerpc64"))]
const TUNSETIFF: libc::c_ulong = 0x800454ca;
#[cfg(not(any(target_arch = "powerpc", target_arch = "powerpc64")))]
const TUNSETIFF: libc::c_ulong = 0x400454ca;
const IFF_TUN: libc::c_short = 0x0001;
const IFF_NO_PI: libc::c_short = 0x1000;

/// A Linux TUN device
pub struct TunDevice {
    fd: RawFd,
    name: String,
}

#[repr(C)]
struct Ifreq {
    ifr_name: [u8; 16],
    ifr_flags: libc::c_short,
    _pad: [u8; 22],
}

impl TunDevice {
    /// Create a new TUN device with the given name
    pub fn create(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Open /dev/net/tun
        let fd = unsafe {
            libc::open(b"/dev/net/tun\0".as_ptr() as *const _, libc::O_RDWR)
        };
        if fd < 0 {
            return Err(format!("Failed to open /dev/net/tun: {}", std::io::Error::last_os_error()).into());
        }

        // Prepare ifreq
        let mut req = Ifreq {
            ifr_name: [0u8; 16],
            ifr_flags: IFF_TUN | IFF_NO_PI,
            _pad: [0u8; 22],
        };
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(15);
        req.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

        // Create the TUN device
        let ret = unsafe { libc::ioctl(fd, TUNSETIFF as _, &mut req as *mut _) };
        if ret < 0 {
            unsafe { libc::close(fd); }
            return Err(format!("ioctl TUNSETIFF failed: {}", std::io::Error::last_os_error()).into());
        }

        // Set non-blocking
        unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };

        let actual_name = std::str::from_utf8(&req.ifr_name)
            .unwrap_or(name)
            .trim_end_matches('\0')
            .to_string();


        Ok(Self { fd, name: actual_name })
    }

    /// Configure the interface with an IP address and bring it up
    pub fn configure(&self, address: &str, subnet: u8, mtu: u16) -> Result<(), Box<dyn std::error::Error>> {
        // Set IP address
        let status = std::process::Command::new("ip")
            .args(["addr", "add", &format!("{}/{}", address, subnet), "dev", &self.name])
            .status()?;
        if !status.success() {
            return Err(format!("Failed to set IP address on {}", self.name).into());
        }

        // Set MTU
        let status = std::process::Command::new("ip")
            .args(["link", "set", "dev", &self.name, "mtu", &mtu.to_string()])
            .status()?;
        if !status.success() {
            warn!("Failed to set MTU on {}", self.name);
        }

        // Bring interface up
        let status = std::process::Command::new("ip")
            .args(["link", "set", "dev", &self.name, "up"])
            .status()?;
        if !status.success() {
            return Err(format!("Failed to bring up {}", self.name).into());
        }

        // Disable IPv6 on the TUN — WolfNet is IPv4-only and the kernel pushes
        // IPv6 multicast (neighbor discovery, IGMP) into the TUN fd, wasting CPU
        let _ = std::fs::write(
            format!("/proc/sys/net/ipv6/conf/{}/disable_ipv6", self.name),
            "1",
        );

        // Disable multicast on the interface — prevents IGMP traffic through the TUN
        let _ = std::process::Command::new("ip")
            .args(["link", "set", "dev", &self.name, "multicast", "off"])
            .status();

        // Tell NetworkManager to ignore this interface — on Fedora/RHEL desktops
        // NM detects the TUN and tries to manage it, messing with routing metrics
        // and causing slow/broken connectivity on WiFi and other interfaces.
        let nm_conf = "/etc/NetworkManager/conf.d/wolfnet.conf";
        if !std::path::Path::new(nm_conf).exists()
            && std::path::Path::new("/etc/NetworkManager/conf.d").is_dir()
        {
            let _ = std::fs::write(nm_conf,
                "# WolfNet: prevent NetworkManager from managing the overlay interface.\n\
                 [keyfile]\n\
                 unmanaged-devices=interface-name:wolfnet*\n");
            let _ = std::process::Command::new("nmcli")
                .args(["general", "reload"]).output();
        }
        let _ = std::process::Command::new("nmcli")
            .args(["device", "set", &self.name, "managed", "no"])
            .output();

        // Prevent Tailscale routing loop — if Tailscale is running, its WireGuard
        // traffic (port 41641) can get routed into wolfnet0 via the subnet route,
        // creating a feedback loop where Tailscale traffic goes through WolfNet
        // and back through Tailscale. Block it with an iptables OUTPUT rule.
        if std::process::Command::new("pidof").arg("tailscaled").output()
            .map(|o| o.status.success()).unwrap_or(false)
        {
            let subnet = format!("{}/{}", address, subnet);
            // Check if the rule already exists before adding
            let exists = std::process::Command::new("iptables")
                .args(["-C", "OUTPUT", "-p", "udp", "--dport", "41641", "-d", &subnet, "-j", "DROP"])
                .output()
                .map(|o| o.status.success()).unwrap_or(false);
            if !exists {
                let _ = std::process::Command::new("iptables")
                    .args(["-I", "OUTPUT", "-p", "udp", "--dport", "41641", "-d", &subnet, "-j", "DROP"])
                    .status();
            }
        }

        Ok(())
    }

    /// Read a packet from the TUN device (blocking if data available)
    /// Returns number of bytes read, or 0 if would block
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(0);
            }
            return Err(err);
        }
        Ok(n as usize)
    }

    /// Write a packet to the TUN device
    pub fn write(&self, data: &[u8]) -> Result<usize, std::io::Error> {
        let n = unsafe { libc::write(self.fd, data.as_ptr() as *const _, data.len()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// Get the raw file descriptor (for poll/select)
    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    /// Get the interface name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set blocking mode
    pub fn set_blocking(&self, blocking: bool) {
        unsafe {
            let flags = libc::fcntl(self.fd, libc::F_GETFL);
            if blocking {
                libc::fcntl(self.fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            } else {
                libc::fcntl(self.fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
    }
}

impl Drop for TunDevice {
    fn drop(&mut self) {

        unsafe { libc::close(self.fd); }
    }
}

/// Extract the destination IPv4 address from a raw IP packet
pub fn get_dest_ip(packet: &[u8]) -> Option<std::net::Ipv4Addr> {
    if packet.len() < 20 {
        return None;
    }
    // IPv4: version in upper nibble of byte 0
    if (packet[0] >> 4) != 4 {
        return None;
    }
    // Destination IP is at offset 16-19
    Some(std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
}

/// Extract the source IPv4 address from a raw IP packet
pub fn get_src_ip(packet: &[u8]) -> Option<std::net::Ipv4Addr> {
    if packet.len() < 20 {
        return None;
    }
    if (packet[0] >> 4) != 4 {
        return None;
    }
    Some(std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]))
}

/// Extract the destination IPv6 address from a raw IP packet. Returns None
/// for non-IPv6 packets or a truncated header. IPv6 has a fixed 40-byte
/// header (RFC 8200 §3); the destination address is the final 16 bytes,
/// offsets 24–39. Used only for v6 subnet routing — the overlay control
/// plane (peer IPs, handshake) stays IPv4.
pub fn get_dest_ip6(packet: &[u8]) -> Option<std::net::Ipv6Addr> {
    if packet.len() < 40 {
        return None;
    }
    // IPv6: version in the upper nibble of byte 0.
    if (packet[0] >> 4) != 6 {
        return None;
    }
    let mut octets = [0u8; 16];
    octets.copy_from_slice(&packet[24..40]);
    Some(std::net::Ipv6Addr::from(octets))
}
