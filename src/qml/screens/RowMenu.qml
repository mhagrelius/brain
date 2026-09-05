import QtQuick
import Brain
import "../components"

// The row's menu: rename and delete for a note; new note, new folder,
// rename and remove for a folder. Opened by right-click or Shift+F10.
Item {
    id: menu
    property var row: null
    visible: row !== null
    z: 50
    anchors.fill: parent

    function openFor(data, at) {
        row = data
        box.x = Math.min(at.x, width - box.width - T.s(8))
        box.y = Math.min(at.y, height - box.height - T.s(8))
    }
    function close() { row = null }

    readonly property var options: {
        if (!row) return []
        if (row.kind === "folder") return [
            {label: "New note here", act: function() { App.newNoteIn(row.path) }},
            {label: "New folder inside", act: function() { App.newFolderIn(row.path) }},
            {label: "Rename folder", act: function() { App.askRenameFolder(row.path) }},
            {label: "Remove folder", danger: true, act: function() { App.askDeleteFolder(row.path) }}
        ]
        return [
            {label: "Open", act: function() { App.openNote(row.id) }},
            {label: "Rename", act: function() { App.askRenameNote(row.id) }},
            {label: "Delete", danger: true, act: function() { App.askDeleteNote(row.id) }}
        ]
    }

    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onClicked: menu.close() }
    Rectangle {
        id: box
        width: T.s(200)
        height: col.height + 2 * T.s(4) + 2 * T.line
        radius: T.s(8)
        color: T.window
        border.width: T.line
        border.color: T.border
        Column {
            id: col
            x: T.s(4); y: T.s(4); width: parent.width - 2 * T.s(4)
            Repeater {
                model: menu.options
                Rectangle {
                    required property var modelData
                    width: parent.width
                    height: T.s(T.menuRowHeight)
                    radius: T.s(6)
                    color: itemHit.containsMouse ? T.selection : "transparent"
                    Sans { x: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.label; px: 13; color: modelData.danger ? T.negative : T.text2 }
                    MouseArea { id: itemHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { var a = modelData.act; menu.close(); a() } }
                }
            }
        }
    }
}
