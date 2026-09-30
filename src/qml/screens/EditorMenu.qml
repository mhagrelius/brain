import QtQuick
import Brain
import "../components"

// The editor's right-click menu, at the pointer: the clipboard verbs, then
// every format the scanner knows with the syntax it writes. Opened by
// NotePane; a format goes through App.applyInsert so it wraps the selection
// that was live when the menu opened.
Item {
    id: menu
    property var editor: null      // the TextEdit
    property bool open: false
    property bool hasSelection: editor && editor.selectionStart !== editor.selectionEnd
    visible: open
    anchors.fill: parent
    z: 40

    property real anchorX: 0
    property real anchorY: 0
    function openAt(p) {
        anchorX = p.x
        anchorY = p.y
        open = true
    }
    function close() { open = false }

    readonly property var clipboard: [
        {label: "Cut", key: "ctrl+x", act: function() { editor.cut() }, needs: true},
        {label: "Copy", key: "ctrl+c", act: function() { editor.copy() }, needs: true},
        {label: "Paste", key: "ctrl+v", act: function() { if (!App.pasteImage()) editor.paste() }},
        {label: "Select all", key: "ctrl+a", act: function() { editor.selectAll() }}
    ]

    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onClicked: menu.close() }
    DropShadow { source: box; anchors.fill: box; menu: true }
    Rectangle {
        id: box
        // Bound, not assigned: the clamp has to follow the box's own height,
        // which is only known once the rows have laid out.
        x: Math.max(T.s(4), Math.min(menu.anchorX, menu.width - width - T.s(4)))
        y: Math.max(T.s(4), Math.min(menu.anchorY, menu.height - height - T.s(4)))
        width: T.s(300)
        height: col.height + 2 * T.s(4) + 2 * T.line
        radius: T.s(8)
        color: T.window
        border.width: T.line
        border.color: T.border
        Column {
            id: col
            x: T.s(4); y: T.s(4); width: parent.width - 2 * T.s(4)
            Repeater {
                model: menu.clipboard
                Rectangle {
                    required property var modelData
                    readonly property bool usable: !modelData.needs || menu.hasSelection
                    width: parent.width; height: T.s(T.menuRowHeight); radius: T.s(6)
                    color: usable && clipHit.containsMouse ? T.selection : "transparent"
                    Sans { x: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.label; px: 13; color: usable ? T.text2 : T.dimmest }
                    Mono { anchors.right: parent.right; anchors.rightMargin: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.key; px: 10; color: T.faint }
                    MouseArea { id: clipHit; anchors.fill: parent; hoverEnabled: true; cursorShape: usable ? Qt.PointingHandCursor : Qt.ArrowCursor; onClicked: { if (!usable) return; var a = modelData.act; menu.close(); a() } }
                }
            }
            Rectangle { width: parent.width; height: T.line; color: T.divider }
            Item { width: 1; height: T.s(2) }
            Repeater {
                model: App.insertRows
                Rectangle {
                    required property var modelData
                    width: parent.width; height: T.s(T.menuRowHeight); radius: T.s(6)
                    color: fmtHit.containsMouse ? T.selection : "transparent"
                    Sans { x: T.s(12); width: T.s(120); anchors.verticalCenter: parent.verticalCenter; text: modelData.label; px: 13; color: T.text2 }
                    Mono { x: T.s(12) + T.s(120); anchors.verticalCenter: parent.verticalCenter; text: modelData.syntax; px: 11.5; color: T.muted }
                    Mono { anchors.right: parent.right; anchors.rightMargin: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.key; px: 10; color: T.faint }
                    MouseArea { id: fmtHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { var label = modelData.label; menu.close(); App.applyInsert(label) } }
                }
            }
        }
    }
    Keys.onEscapePressed: close()
}
