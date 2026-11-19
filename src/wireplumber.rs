pub const RULE_SAVE_PATH: &str =
    "~/.config/wireplumber/wireplumber.conf.d/50-hifi-verify-alsa.conf";

/// Generate a WirePlumber 0.5+ SPA-JSON ALSA rule matching the observed hardware
/// sample rate and quantum/latency.
pub fn generate_rule(device_pattern: &str, rate: Option<u32>, quantum: Option<u32>) -> String {
    let mut props = String::new();
    if let Some(r) = rate {
        props.push_str(&format!("        audio.rate = {}\n", r));
        props.push_str(&format!("        audio.allowed-rates = [ {} ]\n", r));
    }
    if let Some(q) = quantum {
        if let Some(r) = rate {
            props.push_str(&format!("        node.latency = \"{}/{}\"\n", q, r));
        }
        props.push_str(&format!("        api.alsa.period-size = {}\n", q));
    }
    if props.is_empty() {
        props.push_str("        # Start playback first to capture active rate and quantum\n");
    }
    format!(
        "monitor.alsa.rules = [\n  {{\n    matches = [\n      {{ node.name = \"~{}\" }}\n    ]\n    actions = {{\n      update-props = {{\n{}      }}\n    }}\n  }}\n]\n",
        device_pattern, props
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_wireplumber_05_properties() {
        let rule = generate_rule("alsa_output.*", Some(96000), Some(512));
        assert!(rule.contains("audio.rate = 96000"));
        assert!(rule.contains("audio.allowed-rates = [ 96000 ]"));
        assert!(rule.contains("node.latency = \"512/96000\""));
        assert!(rule.contains("api.alsa.period-size = 512"));
        assert!(!rule.contains("node.quantum"));
    }

    #[test]
    fn handles_missing_rate_and_quantum_gracefully() {
        let rule = generate_rule("alsa_output.*", None, None);
        assert!(rule.contains("monitor.alsa.rules"));
        assert!(rule.contains("Start playback first"));
    }
}
