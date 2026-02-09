//! Gateway functionality for WolfNet
//!
//! Enables NAT/masquerading so nodes on the WolfNet can access the internet
//! through a designated gateway node.

use tracing::{info, warn};

/// Detect the default internet-facing interface by parsing the routing table
pub fn detect_external_interface() -> Option<String> {
    let output = std::process::Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse: "default via X.X.X.X dev ethN ..."
    for part in stdout.split_whitespace().collect::<Vec<_>>().windows(2) {
        if part[0] == "dev" {
            return Some(part[1].to_string());
        }
    }
    None
}

/// Enable gateway mode: IP forwarding + NAT masquerading
pub fn enable_gateway(wolfnet_interface: &str, subnet: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ext_iface = detect_external_interface()
        .ok_or("Could not detect external network interface")?;

    info!("Enabling gateway mode: {} -> {} (NAT via {})", wolfnet_interface, ext_iface, subnet);

    // Enable IP forwarding
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")?;
    info!("Enabled IPv4 forwarding");

    // Add iptables MASQUERADE rule for WolfNet traffic going to the internet
    let status = std::process::Command::new("iptables")
        .args(["-t", "nat", "-A", "POSTROUTING", "-s", subnet, "-o", &ext_iface, "-j", "MASQUERADE"])
        .status()?;
    if !status.success() {
        warn!("iptables MASQUERADE rule may have failed");
    }

    // Allow forwarding from wolfnet interface to external
    let status = std::process::Command::new("iptables")
        .args(["-A", "FORWARD", "-i", wolfnet_interface, "-o", &ext_iface, "-j", "ACCEPT"])
        .status()?;
    if !status.success() {
        warn!("iptables FORWARD rule (out) may have failed");
    }

    // Allow established/related traffic back
    let status = std::process::Command::new("iptables")
        .args(["-A", "FORWARD", "-i", &ext_iface, "-o", wolfnet_interface, "-m", "state", "--state", "ESTABLISHED,RELATED", "-j", "ACCEPT"])
        .status()?;
    if !status.success() {
        warn!("iptables FORWARD rule (in) may have failed");
    }

    // Block all other inbound traffic to wolfnet (truly private)
    let status = std::process::Command::new("iptables")
        .args(["-A", "INPUT", "-i", &ext_iface, "-d", subnet, "-j", "DROP"])
        .status()?;
    if !status.success() {
        warn!("iptables INPUT DROP rule may have failed");
    }

    info!("Gateway enabled: WolfNet traffic NAT'd through {}", ext_iface);
    Ok(())
}

/// Clean up gateway rules on shutdown
pub fn disable_gateway(wolfnet_interface: &str, subnet: &str) {
    let ext_iface = detect_external_interface().unwrap_or_default();
    if ext_iface.is_empty() { return; }

    info!("Disabling gateway mode, cleaning up iptables rules");

    let _ = std::process::Command::new("iptables")
        .args(["-t", "nat", "-D", "POSTROUTING", "-s", subnet, "-o", &ext_iface, "-j", "MASQUERADE"])
        .status();
    let _ = std::process::Command::new("iptables")
        .args(["-D", "FORWARD", "-i", wolfnet_interface, "-o", &ext_iface, "-j", "ACCEPT"])
        .status();
    let _ = std::process::Command::new("iptables")
        .args(["-D", "FORWARD", "-i", &ext_iface, "-o", wolfnet_interface, "-m", "state", "--state", "ESTABLISHED,RELATED", "-j", "ACCEPT"])
        .status();
    let _ = std::process::Command::new("iptables")
        .args(["-D", "INPUT", "-i", &ext_iface, "-d", subnet, "-j", "DROP"])
        .status();
}

/// Add a default route through the gateway on a client node
pub fn add_gateway_route(gateway_ip: &str, wolfnet_interface: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("Adding default route via gateway {} on {}", gateway_ip, wolfnet_interface);
    let status = std::process::Command::new("ip")
        .args(["route", "add", "default", "via", gateway_ip, "dev", wolfnet_interface, "metric", "500"])
        .status()?;
    if !status.success() {
        warn!("Failed to add default route via gateway");
    }
    Ok(())
}
