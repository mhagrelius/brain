import QtQuick
import Brain
import "../components"

// One question at a time: a title, an optional field, the one action and a
// way out. Rename, new note, new folder, delete.
Popover {
    id: popup
    widthPx: 460
    Component.onCompleted: if (visible) { field.text = App.prompt.value || ""; field.selectAll(); field.forceActiveFocus() }
    onVisibleChanged: if (visible) { field.text = App.prompt.value || ""; field.forceActiveFocus(); field.selectAll() }

    Column {
        width: parent.width
        Item {
            width: parent.width; height: T.s(14) + titleText.implicitHeight + T.s(14)
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
            Sans { id: titleText; x: T.s(14); anchors.verticalCenter: parent.verticalCenter; width: parent.width - T.s(28); text: App.prompt.title || ""; px: 14; weight: Font.Bold; elide: Text.ElideRight }
        }
        Item {
            width: parent.width
            height: (App.prompt.hasField ? field.height + T.s(14) : 0) + buttons.height + T.s(28)
            Field {
                id: field
                visible: App.prompt.hasField === true
                x: T.s(14); y: T.s(14); width: parent.width - 2 * T.s(14)
                px: 13.5
                onAccepted: App.promptAccept(text)
                Keys.onEscapePressed: App.promptCancel()
                background: Rectangle { color: T.card; radius: T.s(6); border.width: T.line; border.color: field.activeFocus ? T.accent : T.borderStrong }
            }
            Row {
                id: buttons
                anchors.right: parent.right; anchors.rightMargin: T.s(14)
                y: (App.prompt.hasField ? field.y + field.height + T.s(14) : T.s(14))
                spacing: T.s(8)
                SecondaryButton { text: "Cancel"; px: 13; padX: 14; padY: 0; height: T.s(32); fill: T.card; hoverFill: T.track; borderColor: T.borderStrong; fg: T.text2; onClicked: App.promptCancel() }
                PrimaryButton {
                    text: App.prompt.primary || "OK"; px: 13; padX: 14; padY: 0; height: T.s(32)
                    fill: App.prompt.danger ? T.negative : T.accent
                    fg: App.prompt.danger ? T.window : T.accentText
                    onClicked: App.promptAccept(field.text)
                    Keys.onReturnPressed: App.promptAccept(field.text)
                }
            }
        }
    }
    Keys.onPressed: function(e) { if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) { App.promptAccept(field.text); e.accepted = true } else if (e.key === Qt.Key_Escape) { App.promptCancel(); e.accepted = true } }
}
