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
