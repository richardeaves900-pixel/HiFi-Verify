HiFi Verify

A native, Flatpak-first PipeWire verification suite for audiophiles.

HiFi Verify is a ground-up Rust rewrite of an earlier Python audio verifier. It is designed to answer one question definitively: "Is my audio bit-perfect right now, and if not, why?"

Unlike standard system monitors, HiFi Verify reads directly from the hardware (/proc/asound), parses pw-top latency metrics, and visually maps the PipeWire signal path. It is distributed as a Flatpak, using secure host-spawning to read hardware metrics without breaking sandbox isolation.
✨ Features

    Signal Path Matrix: Instantly see if EasyEffects (or any other DSP) is intercepting your audio, and view the exact format your ALSA hardware is receiving.

    Hardware Direct: Reads /proc/asound directly to verify the true DAC format (e.g., S32_LE @ 48.0kHz), bypassing PipeWire's abstractions.

    Latency Jitter Graphing: Plots real-time pw-top WAIT metrics to detect buffer underruns before you hear them.

    CLI & JSON Mode: Sub-100ms CLI commands (--health, --audit, --status) designed for scripting, status bars, and custom automation.

    KDE Plasma Widget: A custom plasmoid that sits in your system tray and displays status via color coding: Green (bit-perfect), Red (compromised/processing), or Gray (idle).

    Sandbox Safe: Fully compliant Flatpak architecture using flatpak-spawn --host to bridge the sandbox for hardware telemetry.

📸 Screenshot
🚀 Installation
Option 1: Flatpak (Recommended)

Since the app is not yet on Flathub, build and install it locally:

    Install required runtimes:
    Bash

    flatpak install flathub org.gnome.Platform//49 org.freedesktop.Sdk.Extension.rust-stable//25.08

    Clone the repository and build:
    Bash

    git clone https://github.com/richardeaves900-pixel/HiFi-Verify.git
    cd HiFi-Verify/flatpak
    flatpak-builder --user --install --force-clean build-dir org.cachyos.HiFiVerify.yaml

    Note: If cargo-sources.json is out of date, regenerate it first:
    Bash

    python3 flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json

Option 2: Build from Source (Host)

Prerequisites: Rust (1.92+), GTK4 development headers, and PipeWire.

    Clone the repository:
    Bash

    git clone https://github.com/richardeaves900-pixel/HiFi-Verify.git
    cd HiFi-Verify

    Build and run:
    Bash

    cargo build --release
    ./target/release/hifi-verify --gui

🏗️ Architecture

HiFi Verify is split into three main layers:

    The Core (src/alsa_proc.rs, src/pwtop.rs): Reads /proc/asound, parses pw-top, and inspects PipeWire graph states.

    The UI (src/ui/): A native GTK4 frontend with Cairo-based rendering for the Signal Path Matrix and latency graphs.

    The Sandbox Bridge (src/host.rs): Automatically detects if running inside Flatpak and uses flatpak-spawn --host to securely request host metrics without requiring broad filesystem permissions.

Flatpak Sandbox Bridge

Because the app is distributed as a Flatpak, it runs in a sandboxed environment without direct access to /proc/asound or unrestricted PipeWire communication.

    src/host.rs: Implements host_cmd and host_popen. When /.flatpak-info is present, it routes commands through flatpak-spawn --host to execute them on the host system.

    ALSA Parsing (src/alsa_proc.rs): Uses host_cmd to locate and read /proc/asound/card*/pcm*p/sub*/hw_params.

Audio Telemetry

    PipeWire Graph (src/telemetry.rs): Parses pw-dump to discover active Stream/Output/Audio nodes, filters out internal EasyEffects nodes to identify the true audio source, and resolves application names via /proc/<pid>/comm.

    pw-top (src/pwtop.rs): Executes pw-top -b -n 2 to parse XRUN counters and WAIT times (jitter).

    MPRIS (src/telemetry.rs): Uses Gio D-Bus bindings to query active media players for codec details and track metadata.

UI Threading Model

GTK4 requires all UI mutations to occur on the main thread:

    A background std::thread polls the audio hardware every 500ms.

    It transmits a Snapshot struct across a std::sync::mpsc channel.

    A glib::timeout_add_local loop drains the channel on the main loop and triggers canvas repaints.
