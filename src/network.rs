use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

pub fn scan_network() -> Vec<String> {
    let mut devices = Vec::new();
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return devices,
    };
    if socket
        .set_read_timeout(Some(Duration::from_millis(1000)))
        .is_err()
    {
        return devices;
    }

    let msg = "M-SEARCH * HTTP/1.1\r\n\
               HOST: 239.255.255.250:1900\r\n\
               MAN: \"ssdp:discover\"\r\n\
               MX: 2\r\n\
               ST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\r\n";

    let target = SocketAddr::from((Ipv4Addr::new(239, 255, 255, 250), 1900));

    if socket.send_to(msg.as_bytes(), target).is_err() {
        return devices;
    }

    let mut buf = [0; 2048];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((amt, src)) => {
                let text = String::from_utf8_lossy(&buf[..amt]);
                for line in text.lines() {
                    if line.to_uppercase().starts_with("LOCATION:") {
                        let url = line
                            .trim_start_matches(|c: char| {
                                c.is_ascii_alphabetic() || c == ':' || c == ' '
                            })
                            .trim();
                        let ip = src.ip();

                        // Fetch the XML to find a friendly name
                        let friendly_name = fetch_friendly_name(url);
                        let display_name =
                            friendly_name.unwrap_or_else(|| "Unknown UPnP Device".to_string());

                        let entry = format!("{} ({})", display_name, ip);
                        if !devices.contains(&entry) {
                            devices.push(entry);
                        }
                    }
                }
            }
            Err(_) => break, // Timeout
        }
    }
    devices
}

fn fetch_friendly_name(url: &str) -> Option<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .new_agent();
    let response = agent.get(url).call().ok()?;
    let text = response.into_body().read_to_string().ok()?;

    // Simple XML parsing without a heavy dependency
    if let Some(start) = text.find("<friendlyName>") {
        if let Some(end) = text.find("</friendlyName>") {
            let name = &text[start + 14..end];
            return Some(name.trim().to_string());
        }
    }
    None
}
