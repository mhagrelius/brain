import QtQuick
import Brain
import "../components"

// Where to look: the vault, the server, what the last pass did, the vectors.
Popover {
    id: panel
    widthPx: 560
    Column {
        width: parent.width
        Item {
            width: parent.width; height: T.s(12) + titleText.implicitHeight + T.s(12)
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
            Sans { id: titleText; x: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: "Sync and search"; px: 14; weight: Font.Bold }
            Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: App.statusInfo.syncing ? "pass running" : "ctrl+shift+s"; px: 11; color: T.faint }
        }
        Column {
            x: T.s(14); width: parent.width - 2 * T.s(14)
            topPadding: T.s(6)
            Repeater {
                model: App.statusInfo.rows || []
                Item {
                    required property var modelData
                    required property int index
                    width: parent.width
                    height: Math.max(T.s(26), valueText.height + T.s(8))
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
                    Mono { anchors.verticalCenter: parent.verticalCenter; text: modelData.k; px: 11; color: T.muted }
                    Mono { id: valueText; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; width: parent.width - T.s(100); horizontalAlignment: Text.AlignRight; text: modelData.v; px: 12; wrapMode: Text.Wrap; color: modelData.c ? T.role(modelData.c) : T.text }
                }
            }
        }
        Item {
            width: parent.width; height: T.s(14) + T.s(32) + T.s(14)
            Row {
                anchors.right: parent.right; anchors.rightMargin: T.s(14); y: T.s(14); spacing: T.s(8)
                SecondaryButton { text: "Close"; px: 13; padX: 14; padY: 0; height: T.s(32); fill: T.card; hoverFill: T.track; borderColor: T.borderStrong; fg: T.text2; onClicked: App.toggleStatus() }
                PrimaryButton { visible: App.statusInfo.hasServer === true; text: "Sync now"; px: 13; padX: 14; padY: 0; height: T.s(32); onClicked: App.syncNow() }
            }
        }
    }
}
