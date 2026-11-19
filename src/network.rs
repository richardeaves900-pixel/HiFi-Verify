use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

fn extract_ssdp_location(response: &str) -> Option<String> {
    for line in response.lines() {
        if let Some((key, val)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("LOCATION") {
                let url = val.trim();
                if url.starts_with("http://") || url.starts_with("https://") {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

fn parse_friendly_name_xml(xml: &str) -> Option<String> {
    const OPEN: &str = "<friendlyName>";
    const CLOSE: &str = "</friendlyName>";
    let start = xml.find(OPEN)? + OPEN.len();
    let rest = xml.get(start..)?;
    let end = rest.find(CLOSE)?;
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn scan_network() -> Vec<String> {
    let mut devices = Vec::new();
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return devices;
    };
    if socket
        .set_read_timeout(Some(Duration::from_millis(400)))
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

    // Collect responses across the full MX: 2 window (plus margin), without
    // letting each packet extend the scan indefinitely.
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut seen_locations = Vec::new();
    let mut buf = [0u8; 4096];

    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((amt, src)) => {
                let text = String::from_utf8_lossy(&buf[..amt]);
                if let Some(url) = extract_ssdp_location(&text) {
                    if !seen_locations.iter().any(|(u, _)| u == &url) {
                        seen_locations.push((url, src.ip()));
                    }
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
    }

    for (url, ip) in seen_locations {
        let display_name =
            fetch_friendly_name(&url).unwrap_or_else(|| "Unknown UPnP Device".to_string());
        let entry = format!("{} ({})", display_name, ip);
        if !devices.contains(&entry) {
            devices.push(entry);
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
    parse_friendly_name_xml(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_full_http_location_without_stripping_scheme() {
        let resp = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.42:8080/description.xml\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n";
        assert_eq!(
            extract_ssdp_location(resp).as_deref(),
            Some("http://192.168.1.42:8080/description.xml")
        );
    }

    #[test]
    fn parses_friendly_name_safely_including_malformed_xml() {
        let ok = "<root><device><friendlyName>WiiM Pro Plus</friendlyName></device></root>";
        assert_eq!(
            parse_friendly_name_xml(ok).as_deref(),
            Some("WiiM Pro Plus")
        );

        let malformed = "</friendlyName><friendlyName>Broken";
        assert_eq!(parse_friendly_name_xml(malformed), None);
    }
}
