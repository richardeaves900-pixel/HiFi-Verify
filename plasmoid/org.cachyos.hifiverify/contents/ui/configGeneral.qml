import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property alias cfg_command: cmdField.text
    property alias cfg_pollIntervalMs: intervalSpin.value
    property alias cfg_showFormat: formatCheck.checked

    Kirigami.FormLayout {
        QQC2.TextField {
            id: cmdField
            Kirigami.FormData.label: "CLI command:"
            placeholderText: "hifi-verify or flatpak run org.cachyos.HiFiVerify"
            Layout.fillWidth: true
        }

        QQC2.SpinBox {
            id: intervalSpin
            Kirigami.FormData.label: "Poll interval (ms):"
            from: 1000
            to: 10000
            stepSize: 500
        }

        QQC2.CheckBox {
            id: formatCheck
            Kirigami.FormData.label: "Panel readout:"
            text: "Show DAC rate and PCM format next to status indicator"
        }
    }
}
