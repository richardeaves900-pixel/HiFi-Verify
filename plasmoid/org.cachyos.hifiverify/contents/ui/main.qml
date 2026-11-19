import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as Plasma5Support
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    property string statusState: "gray"
    property string shortLabel: "STANDBY"
    property string badgeText: "IDLE"
    property string reasonText: "Standby"
    property string appName: "Idle"
    property string trackName: ""
    property string codecName: ""
    property string streamPcm: "—"
    property string dacPcm: "—"
    property string dspSummary: "None detected"
    property int quantum: 0
    property int rateHz: 0

    readonly property string cliCmd: (Plasmoid.configuration.command || "flatpak run org.cachyos.HiFiVerify").trim()
    readonly property string statusCmd: cliCmd + " --status"

    function stateColor(st) {
        if (st === "green") return "#10b981";
        if (st === "red") return "#ef4444";
        if (st === "amber") return "#f59e0b";
        return "#64748b";
    }

    function panelLabel() {
        if (statusState === "gray") {
            return "IDLE";
        }
        if (!Plasmoid.configuration.showFormat) {
            return shortLabel;
        }
        if (statusState === "red") {
            return badgeText + " !";
        }
        if (statusState === "amber") {
            return badgeText + " ?";
        }
        return badgeText;
    }

    toolTipMainText: "HiFi Verify — " + shortLabel
    toolTipSubText: {
        var lines = [reasonText];
        if (statusState !== "gray") {
            lines.push("Player: " + appName + (codecName ? " (" + codecName + ")" : ""));
            if (trackName && trackName !== appName) lines.push("Track: " + trackName);
            lines.push("Stream PCM: " + streamPcm);
            lines.push("DAC Hardware: " + dacPcm);
            lines.push("DSP: " + dspSummary);
        }
        return lines.join("\n");
    }

    // Dedicated one-shot runner (interval: 0) so launching the GUI never re-fires.
    Plasma5Support.DataSource {
        id: launchSource
        engine: "executable"
        interval: 0
        onNewData: function(sourceName, data) {
            disconnectSource(sourceName);
        }
    }

    Plasma5Support.DataSource {
        id: execSource
        engine: "executable"
        connectedSources: [root.statusCmd]
        interval: Math.max(1000, Plasmoid.configuration.pollIntervalMs || 2000)

        onNewData: function(sourceName, data) {
            if (sourceName !== root.statusCmd) {
                disconnectSource(sourceName);
                return;
            }
            var raw = (data["stdout"] || "").trim();
            if (!raw) {
                root.statusState = "gray";
                root.shortLabel = "OFFLINE";
                root.badgeText = "N/A";
                root.reasonText = "Could not query " + root.cliCmd;
                return;
            }
            try {
                var obj = JSON.parse(raw);
                root.statusState = obj.state || "gray";
                root.shortLabel = obj.short_label || "STANDBY";
                root.badgeText = obj.badge || "IDLE";
                root.reasonText = obj.reason || "Standby";
                root.appName = obj.app || "Idle";
                root.trackName = obj.track || "";
                root.codecName = obj.codec || "";
                root.streamPcm = obj.stream_pcm || "—";
                root.dacPcm = obj.dac_pcm || "—";
                root.quantum = obj.quantum || 0;
                root.rateHz = obj.rate_hz || 0;
                if (obj.route_dsp && obj.route_dsp.length > 0) {
                    root.dspSummary = "IN ROUTE: " + obj.route_dsp.join(" → ");
                } else if (obj.processing && obj.processing.length > 0) {
                    root.dspSummary = "Running (bypassed): " + obj.processing.join(", ");
                } else {
                    root.dspSummary = "No PipeWire DSP in route";
                }
            } catch (e) {
                root.statusState = "amber";
                root.shortLabel = "ERROR";
                root.badgeText = "ERR";
                root.reasonText = "Invalid JSON from --status";
            }
        }
    }

    preferredRepresentation: compactRepresentation

    compactRepresentation: MouseArea {
        id: compactRoot
        Layout.minimumWidth: pillRow.implicitWidth + 14
        Layout.preferredWidth: pillRow.implicitWidth + 14
        Layout.fillHeight: true
        hoverEnabled: true
        onClicked: root.expanded = !root.expanded

        Rectangle {
            anchors.centerIn: parent
            width: pillRow.implicitWidth + 12
            height: Math.min(parent.height - 4, pillRow.implicitHeight + 8)
            radius: 6
            color: Qt.rgba(0.05, 0.08, 0.11, 0.85)
            border.width: 1
            border.color: root.stateColor(root.statusState)

            RowLayout {
                id: pillRow
                anchors.centerIn: parent
                spacing: 6

                Rectangle {
                    width: 8
                    height: 8
                    radius: 4
                    color: root.stateColor(root.statusState)
                }

                PlasmaComponents.Label {
                    text: root.panelLabel()
                    font.family: "monospace"
                    font.bold: true
                    font.pixelSize: 11
                    color: "#e4e4e7"
                }
            }
        }
    }

    fullRepresentation: Item {
        Layout.preferredWidth: 340
        Layout.preferredHeight: contentCol.implicitHeight + 24

        ColumnLayout {
            id: contentCol
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Rectangle {
                    width: 11
                    height: 11
                    radius: 5.5
                    color: root.stateColor(root.statusState)
                }

                PlasmaComponents.Label {
                    text: "HiFi Verify — " + root.shortLabel
                    font.bold: true
                    Layout.fillWidth: true
                }

                PlasmaComponents.Label {
                    text: root.badgeText
                    font.family: "monospace"
                    font.bold: true
                    color: root.stateColor(root.statusState)
                }
            }

            PlasmaComponents.Label {
                text: root.reasonText
                wrapMode: Text.WordWrap
                opacity: 0.85
                Layout.fillWidth: true
            }

            Kirigami.Separator {
                Layout.fillWidth: true
            }

            GridLayout {
                columns: 2
                columnSpacing: 10
                rowSpacing: 4
                Layout.fillWidth: true

                PlasmaComponents.Label { text: "Player:"; opacity: 0.7 }
                PlasmaComponents.Label {
                    text: root.appName + (root.codecName ? " · " + root.codecName : "")
                    font.family: "monospace"
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                PlasmaComponents.Label { text: "Stream PCM:"; opacity: 0.7 }
                PlasmaComponents.Label {
                    text: root.streamPcm
                    font.family: "monospace"
                    Layout.fillWidth: true
                }

                PlasmaComponents.Label { text: "DAC Bus:"; opacity: 0.7 }
                PlasmaComponents.Label {
                    text: root.dacPcm
                    font.family: "monospace"
                    Layout.fillWidth: true
                }

                PlasmaComponents.Label { text: "Quantum:"; opacity: 0.7 }
                PlasmaComponents.Label {
                    text: root.quantum > 0 ? ("Q" + root.quantum) : "—"
                    font.family: "monospace"
                    Layout.fillWidth: true
                }

                PlasmaComponents.Label { text: "DSP:"; opacity: 0.7 }
                PlasmaComponents.Label {
                    text: root.dspSummary
                    font.family: "monospace"
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                PlasmaComponents.Button {
                    text: "Open HiFi Verify"
                    icon.name: "audio-card"
                    Layout.fillWidth: true
                    onClicked: {
                        launchSource.connectSource(root.cliCmd + " --gui");
                        root.expanded = false;
                    }
                }

                PlasmaComponents.Button {
                    text: "Refresh"
                    icon.name: "view-refresh"
                    onClicked: {
                        execSource.disconnectSource(root.statusCmd);
                        execSource.connectSource(root.statusCmd);
                    }
                }
            }
        }
    }
}
