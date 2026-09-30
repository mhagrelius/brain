import QtQuick
import Brain
import "../components"

// Notifications and OSD pills, bottom right: a 3px accent bar instead of an
// icon. Success (green), conflict (pink, two buttons), info, and OSD pills
// with a keycap or a progress track.
Column {
    spacing: T.s(10)
    Repeater {
        model: App.toasts
        Rectangle {
            required property var modelData
            readonly property bool osd: modelData.kind === "osd"
            readonly property color accent: modelData.kind === "success" ? T.positive : modelData.kind === "conflict" || modelData.kind === "vanished" ? T.negative : modelData.kind === "info" ? T.muted : T.accent
            width: osd ? osdRow.width + 2 * T.s(14) + 2 * T.line : T.s(400)
            height: osd ? osdRow.height + 2 * T.s(10) + 2 * T.line : body.height + 2 * T.s(14) + 2 * T.line
            anchors.right: parent.right
            radius: T.s(8); color: T.window
            border.width: T.line; border.color: T.border
            DropShadow { source: parent; anchors.fill: parent }

            Row {
                id: osdRow
                visible: osd
                x: T.s(14); y: T.s(10); spacing: T.s(12)
                Sans { text: modelData.title; px: 13; weight: Font.DemiBold; anchors.verticalCenter: parent.verticalCenter }
                Keycap { visible: modelData.progress < 0 && modelData.body.length > 0; text: modelData.body; anchors.verticalCenter: parent.verticalCenter }
                Rectangle {
                    visible: modelData.progress >= 0
                    width: T.s(80); height: T.s(4); radius: T.s(2); color: T.track
                    anchors.verticalCenter: parent.verticalCenter
                    Rectangle { width: parent.width * Math.max(0, Math.min(1, modelData.progress)); height: parent.height; radius: T.s(2); color: T.teal }
                }
                Mono { visible: modelData.progress >= 0; text: modelData.body; px: 11; color: T.teal; anchors.verticalCenter: parent.verticalCenter }
            }

            Rectangle { visible: !osd; x: T.s(14); y: T.s(14); width: T.s(3); height: parent.height - 2 * T.s(14); radius: T.s(2); color: accent }
            Column {
                id: body
                visible: !osd
                x: T.s(14) + T.s(3) + T.s(12); y: T.s(14)
                width: parent.width - x - T.s(14)
                spacing: T.s(4)
                Sans { width: parent.width; text: modelData.title; px: 13.5; weight: Font.Bold; color: modelData.kind === "conflict" || modelData.kind === "vanished" ? T.negative : T.text; wrapMode: Text.Wrap }
                Mono { visible: modelData.body.length > 0; width: parent.width; text: modelData.body; px: 11.5; color: T.meta; wrapMode: Text.Wrap; lineHeight: 1.6 }
                Mono { text: modelData.meta; px: 10; color: T.dimmest }
                Row {
                    visible: modelData.primary.length > 0 || modelData.secondary.length > 0
                    topPadding: T.s(6)
                    spacing: T.s(8)
                    PrimaryButton { visible: modelData.primary.length > 0; text: modelData.primary; px: 12.5; padX: 12; padY: 0; height: T.s(28); onClicked: App.toastAction(modelData.id, "primary") }
                    SecondaryButton { visible: modelData.secondary.length > 0; text: modelData.secondary; px: 12.5; padX: 12; padY: 0; height: T.s(28); fill: T.card; hoverFill: T.track; borderColor: T.borderStrong; fg: T.text2; onClicked: App.toastAction(modelData.id, "secondary") }
                }
            }
            MouseArea { anchors.fill: parent; z: -1; onClicked: if (modelData.primary.length === 0) App.dismissToast(modelData.id) }
        }
    }
}
