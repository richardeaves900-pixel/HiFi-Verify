pub fn generate_rule(device_pattern: &str, rate: Option<u32>, quantum: Option<u32>) -> String {
    let mut props = String::new();
    if let Some(r) = rate {
        props.push_str(&format!("        audio.rate = {}\n", r));
    }
    if let Some(q) = quantum {
        props.push_str(&format!("        node.quantum = \"{}\"\n", q));
        props.push_str(&format!("        node.latency = \"{}\"\n", q));
    }
    format!(
        "monitor.alsa.rules = [\n  {{\n    matches = [\n      {{ node.name = \"~{}\" }}\n    ]\n    actions = {{\n      update-props = {{\n{}      }}\n    }}\n  }}\n]\n",
        device_pattern, props
    )
}
