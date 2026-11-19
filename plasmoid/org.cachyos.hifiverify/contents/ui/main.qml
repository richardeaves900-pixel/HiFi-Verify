import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasma5support as Plasma5Support

PlasmoidItem {
    id: root

    // Live telemetry state populated from `hifi-verify --status`.
    property string statusState: "gray"      // green | amber | red | gray
    property string shortLabel: "STANDBY"
    property string badgeText: "IDLE"
    property string activeOutput: "—"
    property string activeOutputSub: "Standby"
    property string dacFormat: "—"
    property string reasonText: "Standby"
    property string appName: "Standby"
    property string trackName: ""
    property string codecName: ""
    property string streamPcm: "—"
    property string dacPcm: "—"
    property string dspSummary: "No PipeWire DSP in route"
    property int quantum: 0
    property int rateHz: 0
    property int channels: 0

    readonly property bool isDesktop: Plasmoid.formFactor === PlasmaCore.Types.Planar
                                      || Plasmoid.formFactor === PlasmaCore.Types.MediaCenter
    readonly property string cliCmd: (Plasmoid.configuration.command || "flatpak run org.cachyos.HiFiVerify").trim()
    readonly property string statusCmd: cliCmd + " --status"
    readonly property real cardAlpha: Math.max(0.5, Math.min(1.0, (Plasmoid.configuration.backgroundOpacity || 94) / 100.0))

    // Exact HiFi Verify GTK4 palette (`src/ui/style.css`).
    readonly property color colWindow: "#080d11"
    readonly property color colCard: "#0c141b"
    readonly property color colInset: "#0a1118"
    readonly property color colBorder: "#1e293b"
    readonly property color colText: "#e4e4e7"
    readonly property color colMuted: "#94a3b8"
    readonly property color colSub: "#64748b"
    readonly property color colGold: "#fbbf24"
    readonly property color colGoldBorder: "#f59e0b"
    readonly property color colPurple: "#c084fc"
    readonly property color colPurpleBorder: "#a855f7"
    readonly property color colCyan: "#22d3ee"
    readonly property color colCyanBorder: "#06b6d4"
    readonly property color colBtnBg: "#111c24"
    readonly property color colBtnText: "#cbd5e1"

    // Seamless dark desktop card without Plasma's grey frosted frame behind it.
    Plasmoid.backgroundHints: isDesktop
                              ? PlasmaCore.Types.NoBackground
                              : PlasmaCore.Types.DefaultBackground

    function stateColor(st) {
        if (st === "green") return "#10b981";
        if (st === "red") return "#ef4444";
        if (st === "amber") return "#f59e0b";
        return "#64748b";
    }

    function stateGlow(st) {
        if (st === "green") return "#34d399";
        if (st === "red") return "#f87171";
        if (st === "amber") return "#fbbf24";
        return "#94a3b8";
    }

    function stateTint(st) {
        if (st === "green") return Qt.rgba(0.06, 0.72, 0.50, 0.16);
        if (st === "red") return Qt.rgba(0.94, 0.27, 0.27, 0.16);
        if (st === "amber") return Qt.rgba(0.96, 0.62, 0.04, 0.16);
        return Qt.rgba(0.39, 0.45, 0.55, 0.16);
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
            lines.push("Active Output: " + activeOutput + " (" + activeOutputSub + ")");
            lines.push("Player: " + appName + (codecName ? " · " + codecName : ""));
            if (trackName && trackName !== appName) lines.push("Track: " + trackName);
            lines.push("Stream PCM: " + streamPcm);
            lines.push("DAC Bus: " + dacPcm);
            lines.push("DSP: " + dspSummary);
        }
        return lines.join("\n");
    }

    // One-shot runner (interval: 0) so launching the main GUI never re-fires.
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
        interval: Math.max(1000, Plasmoid.configuration.pollIntervalMs || 2500)

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
                root.activeOutput = "—";
                root.activeOutputSub = "CLI unreachable";
                root.dacFormat = "—";
                root.appName = "Offline";
                root.reasonText = "Could not query " + root.cliCmd;
                return;
            }
            try {
                var obj = JSON.parse(raw);
                root.statusState = obj.state || "gray";
                root.shortLabel = obj.short_label || "STANDBY";
                root.badgeText = obj.badge || "IDLE";
                root.activeOutput = obj.active_output || (root.statusState === "gray" ? "—" : root.badgeText);
                root.activeOutputSub = obj.active_output_sub || (root.statusState === "gray" ? "Standby" : "");
                root.dacFormat = obj.dac_format || "—";
                root.reasonText = obj.reason || "Standby";
                root.appName = obj.app || "Standby";
                root.trackName = obj.track || "";
                root.codecName = obj.codec || "";
                root.streamPcm = obj.stream_pcm || "—";
                root.dacPcm = obj.dac_pcm || "—";
                root.quantum = obj.quantum || 0;
                root.rateHz = obj.rate_hz || 0;
                root.channels = obj.channels || 0;
                if (obj.route_dsp && obj.route_dsp.length > 0) {
                    root.dspSummary = "IN ROUTE: " + obj.route_dsp.join(" → ");
                } else if (obj.processing && obj.processing.length > 0) {
                    root.dspSummary = "Bypassed: " + obj.processing.join(", ");
                } else {
                    root.dspSummary = "No PipeWire DSP in route";
                }
            } catch (e) {
                root.statusState = "amber";
                root.shortLabel = "ERROR";
                root.badgeText = "ERR";
                root.activeOutput = "—";
                root.activeOutputSub = "Parse error";
                root.reasonText = "Invalid JSON from --status";
            }
        }
    }

    // Automatically render the themed glance card directly on the Desktop,
    // and the compact pill when docked in a Panel.
    preferredRepresentation: isDesktop ? fullRepresentation : compactRepresentation

    // ---- Panel Compact Pill ---------------------------------------------
    compactRepresentation: MouseArea {
        id: compactRoot
        Layout.minimumWidth: pillRow.implicitWidth + 16
        Layout.preferredWidth: pillRow.implicitWidth + 16
        Layout.fillHeight: true
        hoverEnabled: true
        onClicked: root.expanded = !root.expanded

        Rectangle {
            anchors.centerIn: parent
            width: pillRow.implicitWidth + 14
            height: Math.min(parent.height - 4, pillRow.implicitHeight + 8)
            radius: 6
            color: Qt.rgba(0.04, 0.07, 0.09, 0.92)
            border.width: 1
            border.color: root.stateColor(root.statusState)

            RowLayout {
                id: pillRow
                anchors.centerIn: parent
                spacing: 6

                Item {
                    width: 10
                    height: 10
                    Rectangle {
                        anchors.centerIn: parent
                        width: 10
                        height: 10
                        radius: 5
                        color: root.stateGlow(root.statusState)
                        opacity: 0.28
                    }
                    Rectangle {
                        anchors.centerIn: parent
                        width: 6
                        height: 6
                        radius: 3
                        color: root.stateColor(root.statusState)
                    }
                }

                Text {
                    text: root.panelLabel()
                    font.family: "monospace"
                    font.bold: true
                    font.pixelSize: 11
                    color: root.statusState === "gray" ? root.colMuted : root.colGold
                }
            }
        }
    }

    // ---- Desktop Glance Widget & Panel Popup ----------------------------
    fullRepresentation: Item {
        id: fullRoot

        readonly property bool showExtraCards: !Plasmoid.configuration.compactDesktop
                                               && height >= 195

        Layout.minimumWidth: 255
        Layout.minimumHeight: Plasmoid.configuration.compactDesktop ? 132 : 145
        Layout.preferredWidth: 332
        Layout.preferredHeight: Plasmoid.configuration.compactDesktop ? 148 : 232

        Rectangle {
            id: shell
            anchors.fill: parent
            radius: 10
            color: Qt.rgba(root.colWindow.r, root.colWindow.g, root.colWindow.b, root.cardAlpha)
            border.width: 1.5
            border.color: root.statusState === "gray" ? root.colBorder : root.stateColor(root.statusState)

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 9
                spacing: 7

                // Header Bar: Brand + Glowing 4-State Status LED Pill
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 30
                    radius: 6
                    color: root.colInset
                    border.width: 1
                    border.color: root.colBorder

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 9
                        anchors.rightMargin: 7
                        spacing: 6

                        Text {
                            text: "HiFi Verify"
                            font.family: "monospace"
                            font.bold: true
                            font.pixelSize: 11
                            color: root.colText
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        // Red / Amber / Green / Grey Status Indicator Pill
                        Rectangle {
                            radius: 5
                            color: root.stateTint(root.statusState)
                            border.width: 1
                            border.color: root.stateColor(root.statusState)
                            implicitHeight: 20
                            implicitWidth: statusPillRow.implicitWidth + 12

                            RowLayout {
                                id: statusPillRow
                                anchors.centerIn: parent
                                spacing: 5

                                Item {
                                    width: 10
                                    height: 10
                                    Rectangle {
                                        anchors.centerIn: parent
                                        width: 10
                                        height: 10
                                        radius: 5
                                        color: root.stateGlow(root.statusState)
                                        opacity: 0.35
                                    }
                                    Rectangle {
                                        anchors.centerIn: parent
                                        width: 6
                                        height: 6
                                        radius: 3
                                        color: root.stateColor(root.statusState)
                                    }
                                }

                                Text {
                                    text: root.shortLabel
                                    font.family: "monospace"
                                    font.bold: true
                                    font.pixelSize: 10
                                    color: root.stateGlow(root.statusState)
                                }
                            }
                        }
                    }
                }

                // Hero Card: ACTIVE OUTPUT RATE (Matches main GTK4 Gold Card)
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: !fullRoot.showExtraCards
                    implicitHeight: 68
                    radius: 6
                    color: root.colCard
                    border.width: 1
                    border.color: root.statusState === "gray" ? root.colBorder : root.colGoldBorder

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 2

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 6

                            Text {
                                text: "ACTIVE OUTPUT RATE"
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 10
                                color: root.colMuted
                                Layout.fillWidth: true
                            }

                            // Prominent secondary status LED dot inside the card corner
                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: root.stateColor(root.statusState)
                            }
                        }

                        Text {
                            text: root.activeOutput
                            font.family: "monospace"
                            font.bold: true
                            font.pixelSize: 22
                            fontSizeMode: Text.HorizontalFit
                            minimumPixelSize: 14
                            color: root.statusState === "gray" ? root.colSub : root.colGold
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        Text {
                            text: root.activeOutputSub
                            font.family: "monospace"
                            font.pixelSize: 10
                            color: root.statusState === "gray" ? root.colSub : root.colMuted
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }
                    }
                }

                // Mini Telemetry Row: Player (Purple) + DAC Bus (Cyan)
                RowLayout {
                    visible: fullRoot.showExtraCards
                    Layout.fillWidth: true
                    spacing: 6

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        implicitHeight: 54
                        radius: 6
                        color: root.colCard
                        border.width: 1
                        border.color: root.statusState === "gray" ? root.colBorder : root.colPurpleBorder

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 7
                            spacing: 2

                            Text {
                                text: "PLAYER"
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 10
                                color: root.colMuted
                            }
                            Text {
                                text: root.appName
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 13
                                color: root.statusState === "gray" ? root.colSub : root.colPurple
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                            Text {
                                text: root.trackName && root.trackName !== root.appName
                                      ? root.trackName
                                      : (root.codecName || "No active stream")
                                font.family: "monospace"
                                font.pixelSize: 10
                                color: root.colSub
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        implicitHeight: 54
                        radius: 6
                        color: root.colCard
                        border.width: 1
                        border.color: root.statusState === "gray" ? root.colBorder : root.colCyanBorder

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 7
                            spacing: 2

                            Text {
                                text: "DAC BUS"
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 10
                                color: root.colMuted
                            }
                            Text {
                                text: root.dacFormat
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 13
                                color: root.statusState === "gray" ? root.colSub : root.colCyan
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                            Text {
                                text: root.quantum > 0
                                      ? ("Q" + root.quantum + (root.channels > 0 ? " • " + root.channels + "ch" : ""))
                                      : root.dspSummary
                                font.family: "monospace"
                                font.pixelSize: 10
                                color: root.colSub
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                        }
                    }
                }

                // Footer Strip: Path Status Detail + Quick Launch / Refresh
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 28
                    radius: 6
                    color: root.colInset
                    border.width: 1
                    border.color: root.colBorder

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 5
                        spacing: 6

                        Text {
                            text: root.reasonText
                            font.family: "monospace"
                            font.pixelSize: 10
                            color: root.stateGlow(root.statusState)
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        // Cyber-styled Open App button
                        Rectangle {
                            id: openBtn
                            implicitHeight: 20
                            implicitWidth: openLbl.implicitWidth + 12
                            radius: 5
                            color: root.colBtnBg
                            border.width: 1
                            border.color: openMouse.containsMouse ? root.colCyan : root.colBorder

                            Text {
                                id: openLbl
                                anchors.centerIn: parent
                                text: "Open"
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 10
                                color: openMouse.containsMouse ? root.colCyan : root.colBtnText
                            }

                            MouseArea {
                                id: openMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    launchSource.connectSource(root.cliCmd + " --gui");
                                    if (!root.isDesktop) {
                                        root.expanded = false;
                                    }
                                }
                            }
                        }

                        // Cyber-styled Refresh button
                        Rectangle {
                            id: refreshBtn
                            implicitHeight: 20
                            implicitWidth: 22
                            radius: 5
                            color: root.colBtnBg
                            border.width: 1
                            border.color: refreshMouse.containsMouse ? root.colCyan : root.colBorder

                            Text {
                                anchors.centerIn: parent
                                text: "↻"
                                font.family: "monospace"
                                font.bold: true
                                font.pixelSize: 11
                                color: refreshMouse.containsMouse ? root.colCyan : root.colBtnText
                            }

                            MouseArea {
                                id: refreshMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    execSource.disconnectSource(root.statusCmd);
                                    execSource.connectSource(root.statusCmd);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
