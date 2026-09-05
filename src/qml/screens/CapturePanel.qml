import QtQuick
import Brain
import "../components"

// Quick capture: one line into Inbox.md (or the open note). Return saves,
// Ctrl+Return saves and opens, Tab cycles the destination, Esc discards.
Popover {
    id: panel
    widthPx: 560
    property var candidates: []
    property int pick: 0

    function linkQuery() {
        var t = input.text.substring(0, input.cursorPosition)
        var open = t.lastIndexOf("[[")
        if (open < 0) return null
        var q = t.substring(open + 2)
        if (q.indexOf("]]") >= 0 || q.indexOf("\n") >= 0) return null
        return q
    }
    function refresh() { var q = linkQuery(); candidates = q === null ? [] : App.linkCandidates(q); pick = 0 }
    function accept(title) {
        var t = input.text.substring(0, input.cursorPosition)
        var open = t.lastIndexOf("[[")
        var before = input.text.substring(0, open + 2)
        var after = input.text.substring(input.cursorPosition)
        input.text = before + title + "]]" + after
        input.cursorPosition = before.length + title.length + 2
        candidates = []
    }
    onVisibleChanged: if (visible) { input.text = ""; candidates = []; input.forceActiveFocus() }
    Component.onCompleted: if (visible) input.forceActiveFocus()

    Column {
        width: parent.width
        Item {
            width: parent.width
            height: T.s(12) + capTitle.implicitHeight + T.s(12)
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
            Sans { id: capTitle; x: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: "Capture"; px: 14; weight: Font.Bold }
            Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: "→ " + App.captureDestination; px: 11; color: T.faint }
        }
        Item {
            width: parent.width
            height: box.height + 2 * T.s(14) + (candidates.length ? list.height + T.s(6) : 0)
            Rectangle {
                id: box
                x: T.s(14); y: T.s(14); width: parent.width - 2 * T.s(14)
                height: Math.max(T.s(32), input.contentHeight + 2 * T.s(8))
                radius: T.s(6); color: T.card
                border.width: T.line; border.color: input.activeFocus ? T.accent : T.borderStrong
                Mono { x: T.s(10); y: T.s(8); text: "+"; px: 13; color: T.accent }
                TextEdit {
                    id: input
                    x: T.s(26); y: T.s(8); width: parent.width - x - T.s(10)
                    font.family: T.sans; font.pointSize: T.f(13.5)
                    color: T.text; selectionColor: T.activeFill; selectedTextColor: T.activeText
                    wrapMode: TextEdit.Wrap
                    cursorDelegate: Rectangle { width: T.s(2); color: T.accent }
                    onTextChanged: panel.refresh()
                    onCursorPositionChanged: panel.refresh()
                    Keys.onPressed: function(e) {
                        if (panel.candidates.length) {
                            if (e.key === Qt.Key_Down) { panel.pick = Math.min(panel.candidates.length - 1, panel.pick + 1); e.accepted = true; return }
                            if (e.key === Qt.Key_Up) { panel.pick = Math.max(0, panel.pick - 1); e.accepted = true; return }
                            if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter || e.key === Qt.Key_Tab) { panel.accept(panel.candidates[panel.pick]); e.accepted = true; return }
                        }
                        if (e.key === Qt.Key_Escape) { App.closeCapture(); e.accepted = true; return }
                        if (e.key === Qt.Key_Tab) { App.captureCycleDestination(); e.accepted = true; return }
                        if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) { App.capture(text, (e.modifiers & Qt.ControlModifier) !== 0); e.accepted = true }
                    }
                }
            }
            Rectangle {
                id: list
                visible: panel.candidates.length > 0
                x: T.s(14); y: box.y + box.height + T.s(6); width: box.width
                height: listCol.height + 2 * T.line
                radius: T.s(6); color: T.card; border.width: T.line; border.color: T.divider
                Column {
                    id: listCol
                    x: T.line; y: T.line; width: parent.width - 2 * T.line
                    Repeater {
                        model: panel.candidates
                        Rectangle {
                            required property string modelData
                            required property int index
                            width: parent.width; height: T.s(30)
                            color: index === panel.pick ? T.selection : "transparent"
                            Sans { x: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData; px: 13 }
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: panel.accept(modelData) }
                        }
                    }
                }
            }
        }
        Item {
            width: parent.width; height: T.s(28)
            Rectangle { width: parent.width; height: T.line; color: T.divider }
            Row {
                x: T.s(14); anchors.verticalCenter: parent.verticalCenter; spacing: T.s(16)
                Mono { text: "return save"; px: 10; color: T.dimmest }
                Mono { text: "ctrl+return save + open"; px: 10; color: T.dimmest }
                Mono { text: "tab destination"; px: 10; color: T.dimmest }
                Mono { text: "esc discard"; px: 10; color: T.dimmest }
            }
        }
    }
}
