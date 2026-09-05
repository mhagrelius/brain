import QtQuick
import QtQuick.Window
import Brain
import "components"
import "screens"

// Hyprland draws the border; the app owns everything inside. Three columns:
// sidebar 290 · content · rail 360, a status rail under the content, and the
// summoned surfaces over the lot.
Window {
    id: win
    width: Math.round(1440 * T.scale)
    height: Math.round(900 * T.scale)
    visible: true
    title: "Brain" + (App.hasNote ? " — " + App.noteTitle : "")
    color: T.window

    readonly property bool typing: activeFocusItem && (activeFocusItem instanceof TextInput || activeFocusItem instanceof TextEdit)
    readonly property bool surfaceOpen: App.captureOpen || App.insertOpen || App.searchOpen || App.statusOpen || App.prompt.kind !== undefined

    function focusEditor() { if (App.hasNote && !App.reading) note.focusEditor(); else root.forceActiveFocus() }
    function closeSurfaces() {
        if (App.captureOpen) App.closeCapture()
        if (App.insertOpen) App.closeInsert()
        if (App.searchOpen) App.closeSearch()
        if (App.statusOpen) App.toggleStatus()
        if (App.prompt.kind !== undefined) App.promptCancel()
    }

    Connections {
        target: App
        function onEditorFocusRequested() { Qt.callLater(win.focusEditor) }
        function onSearchFocusRequested() { note.focusSearch() }
    }

    Shortcut { sequence: "Ctrl+Q"; context: Qt.ApplicationShortcut; onActivated: win.close() }
    Shortcut { sequence: "Ctrl+N"; context: Qt.ApplicationShortcut; onActivated: { win.closeSurfaces(); App.newNote() } }
    Shortcut { sequence: "Ctrl+K"; context: Qt.ApplicationShortcut; onActivated: { win.closeSurfaces(); App.openSearch("titles") } }
    Shortcut { sequence: "Ctrl+Shift+F"; context: Qt.ApplicationShortcut; onActivated: { win.closeSurfaces(); App.openSearch("text") } }
    Shortcut { sequence: "Ctrl+F"; context: Qt.ApplicationShortcut; onActivated: { win.closeSurfaces(); note.focusSearch() } }
    Shortcut { sequence: "Ctrl+E"; context: Qt.ApplicationShortcut; onActivated: App.toggleReading() }
    Shortcut { sequence: "Ctrl+Space"; context: Qt.ApplicationShortcut; onActivated: { if (App.insertOpen) App.closeInsert(); else { win.closeSurfaces(); note.rememberSelection(); App.openInsert() } } }
    Shortcut { sequences: ["Meta+N", "Ctrl+Shift+N"]; context: Qt.ApplicationShortcut; onActivated: { win.closeSurfaces(); App.openCapture() } }
    Shortcut { sequence: "Ctrl+S"; context: Qt.ApplicationShortcut; onActivated: App.saveNow() }
    Shortcut { sequence: "Ctrl+Shift+S"; context: Qt.ApplicationShortcut; onActivated: App.toggleStatus() }
    Shortcut { sequence: "Ctrl+B"; context: Qt.ApplicationShortcut; onActivated: note.format("Bold") }
    Shortcut { sequence: "Ctrl+I"; context: Qt.ApplicationShortcut; onActivated: note.format("Italic") }
    Shortcut { sequence: "Ctrl+L"; context: Qt.ApplicationShortcut; onActivated: note.format("Link to Note") }
    Shortcut { sequence: "Ctrl+Shift+T"; context: Qt.ApplicationShortcut; onActivated: note.format("Task") }

    Item {
        id: root
        anchors.fill: parent
        focus: true
        Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) { win.closeSurfaces(); event.accepted = true; return }
            if (win.typing || win.surfaceOpen) return
            if (event.modifiers & (Qt.ControlModifier | Qt.MetaModifier | Qt.AltModifier)) return
            if (event.key >= Qt.Key_1 && event.key <= Qt.Key_3) { App.goToKey(event.key - Qt.Key_0); event.accepted = true; return }
            if (event.text === "/") { note.focusSearch(); event.accepted = true }
        }

        Sidebar {
            id: sidebar
            height: parent.height
            visible: App.hasVault
        }
        Rectangle { x: sidebar.width; width: T.line; height: parent.height; color: T.divider; visible: App.hasVault }

        Item {
            id: content
            x: App.hasVault ? sidebar.width + T.line : 0
            width: parent.width - x
            height: parent.height

            FirstRun { anchors.fill: parent; visible: !App.hasVault }

            Item {
                anchors.fill: parent
                visible: App.hasVault
                NotePane {
                    id: note
                    x: 0
                    width: parent.width - rail.width - T.line
                    height: parent.height - status.height
                }
                Rectangle { x: note.width; width: T.line; height: note.height; color: T.divider }
                Rail {
                    id: rail
                    x: note.width + T.line
                    width: T.s(T.railWidth)
                    height: note.height
                }
                // Status rail: 28px, mono 11, a dot in the sync colour.
                Rectangle {
                    id: status
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: T.s(28)
                    color: T.window
                    Rectangle { width: parent.width; height: T.line; color: T.divider }
                    Mono { x: T.s(16); anchors.verticalCenter: parent.verticalCenter; width: parent.width - T.s(32) - rightRow.width - T.s(12); text: App.statusLeft; px: 11; color: T.faint; elide: Text.ElideRight }
                    Row {
                        id: rightRow
                        anchors.right: parent.right; anchors.rightMargin: T.s(16)
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: T.s(6)
                        Mono { text: App.statusRight; px: 11; color: T.faint; anchors.verticalCenter: parent.verticalCenter }
                        Mono { text: "●"; px: 11; color: T.role(App.statusDot); anchors.verticalCenter: parent.verticalCenter }
                    }
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.toggleStatus() }
                }
            }
        }

        // ---- summoned surfaces ------------------------------------------------
        MouseArea {
            anchors.fill: parent
            visible: win.surfaceOpen
            onClicked: win.closeSurfaces()
            Rectangle { anchors.fill: parent; color: Qt.rgba(0, 0, 0, 0.18) }
        }
        CapturePanel { x: Math.round((parent.width - width) / 2); y: T.s(120); visible: App.captureOpen }
        InsertMenu { x: Math.round((parent.width - width) / 2); y: T.s(120); visible: App.insertOpen }
        SearchPanel { x: Math.round((parent.width - width) / 2); y: T.s(100); visible: App.searchOpen }
        StatusPanel { x: Math.round((parent.width - width) / 2); y: T.s(140); visible: App.statusOpen }
        PromptPopup { x: Math.round((parent.width - width) / 2); y: T.s(160); visible: App.prompt.kind !== undefined }
        Toasts { anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: T.s(18) }
        Tooltip { hostWidth: root.width; hostHeight: root.height }
    }
}
