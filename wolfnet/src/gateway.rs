//! Gateway functionality for WolfNet
//!
//! Enables NAT/masquerading so nodes on the WolfNet can access the internet
//! through a designated gateway node.

use tracing::warn;

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
/// Add an iptables rule unless it is already present.
///
/// These rules used to be appended unconditionally, so every WolfNet restart
/// left another identical copy behind — and `disable_gateway` only ever removes
/// one of each. A gateway node that had been upgraded a dozen times carried a
/// dozen stale MASQUERADE and FORWARD rules. `-C` is the check form of the same
/// rule: present means there is nothing to do.
fn ensure_rule(args: &[&str]) -> bool {
    let check: Vec<&str> = args.iter()
        .map(|a| if *a == "-A" { "-C" } else { *a })
        .collect();
    let already_present = std::process::Command::new("iptables")
        .args(&check)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if already_present {
        return true;
    }
    std::process::Command::new("iptables")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn enable_gateway(wolfnet_interface: &str, subnet: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ext_iface = detect_external_interface()
        .ok_or("Could not detect external network interface")?;



    // Enable forwarding only on the WolfNet and external interfaces — avoid
    // global ip_forward which turns the machine into a full router and can
    // cause network-wide slowdowns (especially on low-powered devices like Pis).
    let _ = std::process::Command::new("sysctl")
        .args(["-w", &format!("net.ipv4.conf.{}.forwarding=1", wolfnet_interface)]).output();
    let _ = std::process::Command::new("sysctl")
        .args(["-w", &format!("net.ipv4.conf.{}.forwarding=1", ext_iface)]).output();
    // Disable ICMP redirects on these interfaces
    let _ = std::process::Command::new("sysctl")
        .args(["-w", &format!("net.ipv4.conf.{}.send_redirects=0", wolfnet_interface)]).output();
    let _ = std::process::Command::new("sysctl")
        .args(["-w", &format!("net.ipv4.conf.{}.send_redirects=0", ext_iface)]).output();


    // Add iptables MASQUERADE rule for WolfNet traffic going to the internet
    if !ensure_rule(&["-t", "nat", "-A", "POSTROUTING", "-s", subnet, "-o", &ext_iface, "-j", "MASQUERADE"]) {
        warn!("iptables MASQUERADE rule may have failed");
    }

    // Allow forwarding from wolfnet interface to external
    if !ensure_rule(&["-A", "FORWARD", "-i", wolfnet_interface, "-o", &ext_iface, "-j", "ACCEPT"]) {
        warn!("iptables FORWARD rule (out) may have failed");
    }

    // Allow established/related traffic back
    if !ensure_rule(&["-A", "FORWARD", "-i", &ext_iface, "-o", wolfnet_interface, "-m", "state", "--state", "ESTABLISHED,RELATED", "-j", "ACCEPT"]) {
        warn!("iptables FORWARD rule (in) may have failed");
    }

    // Removed in v20.11.9: an `-A INPUT -i <ext> -d <subnet> -j DROP` rule
    // used to live here under the banner "Block all other inbound traffic
    // to wolfnet (truly private)". klasSponsor (2026-04-27) confirmed it
    // was breaking WolfRouter subnet routing on multiple nodes — peers'
    // packets and replies that legitimately transited via the WolfNet
    // CGNAT range were getting dropped depending on the path.
    //
    // The rule also wasn't actually defending anything: a packet on the
    // WAN destined for an RFC1918/CGNAT IP can only reach INPUT if that
    // IP is local to this host (i.e. its wolfnet0 address) — and packets
    // from the public internet to a host's wolfnet0 IP can't actually
    // arrive without spoofing or a misrouted upstream, neither of which
    // the rule meaningfully mitigates. WolfNet privacy comes from the
    // encrypted overlay itself, not from filtering at the gateway's
    // INPUT chain.
    //
    // Belt-and-braces: proactively delete any copy of the old rule that's
    // still installed on existing nodes from previous releases, so an
    // upgrade-and-restart of WolfNet quietly cleans them up. Repeated
    // -D runs handle the case where multiple copies got appended on
    // earlier daemon reloads.
    for _ in 0..4 {
        let status = std::process::Command::new("iptables")
            .args(["-D", "INPUT", "-i", &ext_iface, "-d", subnet, "-j", "DROP"])
            .status();
        match status {
            Ok(s) if s.success() => continue, // deleted one, try again
            _ => break,                       // none left (or iptables missing)
        }
    }

    Ok(())
}

/// Clean up gateway rules on shutdown
pub fn disable_gateway(wolfnet_interface: &str, subnet: &str) {
    let ext_iface = detect_external_interface().unwrap_or_default();
    if ext_iface.is_empty() { return; }



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

    let status = std::process::Command::new("ip")
        .args(["route", "add", "default", "via", gateway_ip, "dev", wolfnet_interface, "metric", "500"])
        .status()?;
    if !status.success() {
        warn!("Failed to add default route via gateway");
    }
    Ok(())
}
