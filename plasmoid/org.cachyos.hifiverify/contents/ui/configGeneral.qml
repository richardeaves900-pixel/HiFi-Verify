import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property alias cfg_command: cmdField.text
    property alias cfg_pollIntervalMs: intervalSpin.value
    property alias cfg_showFormat: formatCheck.checked
    property alias cfg_compactDesktop: compactDesktopCheck.checked
    property alias cfg_backgroundOpacity: opacitySpin.value

    Kirigami.FormLayout {
        QQC2.TextField {
            id: cmdField
            Kirigami.FormData.label: "CLI command:"
            placeholderText: "flatpak run org.cachyos.HiFiVerify or hifi-verify"
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
            text: "Show Active Output rate next to status LED in panel"
        }

        QQC2.CheckBox {
            id: compactDesktopCheck
            Kirigami.FormData.label: "Desktop layout:"
            text: "Compact glance card (Active Output + Status LED only)"
        }

        QQC2.SpinBox {
            id: opacitySpin
            Kirigami.FormData.label: "Card opacity (%):"
            from: 50
            to: 100
            stepSize: 5
        }
    }
}
