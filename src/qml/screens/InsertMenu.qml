import QtQuick
import Brain
import "../components"

// The insert menu: every format the scanner knows, filtered as you type,
// each showing the syntax it writes. Replaces the old formatting grid.
Popover {
    id: menu
    widthPx: 520
    property int pick: 0
    readonly property var all: App.insertRows.concat(App.recentRows)
    Component.onCompleted: if (visible) { filter.text = App.insertFilterText; filter.forceActiveFocus() }
    onVisibleChanged: if (visible) { filter.text = ""; pick = 0; filter.forceActiveFocus() }
    Connections { target: App; function onChanged() { if (menu.pick >= menu.all.length) menu.pick = 0 } }

    function apply(row) { App.applyInsert(row.label) }
    function boldLabel(row) {
        var out = ""
        var pos = row.positions || []
        for (var i = 0; i < row.label.length; ++i) {
            var c = row.label[i]
            out += pos.indexOf(i) >= 0 ? "<b><font color='" + T.text + "'>" + c + "</font></b>" : c
        }
        return out
    }

    Column {
        width: parent.width
        Item {
            width: parent.width; height: T.s(12) + box.height + T.s(12)
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
            Rectangle {
                id: box
                x: T.s(14); y: T.s(12); width: parent.width - 2 * T.s(14); height: T.s(30)
                radius: T.s(6); color: T.card; border.width: T.line; border.color: filter.activeFocus ? T.accent : T.borderStrong
                Mono { x: T.s(10); anchors.verticalCenter: parent.verticalCenter; text: "/"; px: 12; color: T.accent }
                TextInput {
                    id: filter
                    x: T.s(24); width: parent.width - x - hint.width - T.s(20)
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: T.sans; font.pointSize: T.f(13)
                    color: T.text; selectionColor: T.activeFill; selectedTextColor: T.activeText
                    cursorDelegate: Rectangle { width: T.s(2); color: T.accent }
                    onTextEdited: { App.insertFilter(text); menu.pick = 0 }
                    Keys.onPressed: function(e) {
                        if (e.key === Qt.Key_Escape) { App.closeInsert(); e.accepted = true }
                        else if (e.key === Qt.Key_Down) { menu.pick = Math.min(menu.all.length - 1, menu.pick + 1); e.accepted = true }
                        else if (e.key === Qt.Key_Up) { menu.pick = Math.max(0, menu.pick - 1); e.accepted = true }
                        else if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) { if (menu.all.length) menu.apply(menu.all[menu.pick]); e.accepted = true }
                    }
                }
                Mono { id: hint; anchors.right: parent.right; anchors.rightMargin: T.s(10); anchors.verticalCenter: parent.verticalCenter; text: "wraps the selection"; px: 10; color: T.faint }
            }
        }
        Item { width: 1; height: T.s(4) }
        Repeater {
            model: App.insertRows
            Rectangle {
                required property var modelData
                required property int index
                width: parent.width; height: T.s(30)
                color: index === menu.pick ? T.selection : rowHit.containsMouse ? T.card : "transparent"
                Sans { x: T.s(14); width: T.s(130); anchors.verticalCenter: parent.verticalCenter; textFormat: Text.RichText; text: menu.boldLabel(modelData); px: 13; color: T.text2 }
                Mono { x: T.s(14) + T.s(130); anchors.verticalCenter: parent.verticalCenter; text: modelData.syntax; px: 12; color: T.muted }
                Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: modelData.key; px: 10; color: T.faint }
                MouseArea { id: rowHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: menu.apply(modelData) }
            }
        }
        Rectangle { visible: App.recentRows.length > 0; width: parent.width; height: T.line; color: T.divider }
        Mono { visible: App.recentRows.length > 0; x: T.s(14); topPadding: T.s(10); bottomPadding: T.s(6); text: "Recent"; px: 10; color: T.faint; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(1.6) }
        Repeater {
            model: App.recentRows
            Rectangle {
                required property var modelData
                required property int index
                width: parent.width; height: T.s(30)
                color: index + App.insertRows.length === menu.pick ? T.selection : recentHit.containsMouse ? T.card : "transparent"
                Sans { x: T.s(14); width: T.s(130); anchors.verticalCenter: parent.verticalCenter; text: modelData.label; px: 13; color: T.text2 }
                Mono { x: T.s(14) + T.s(130); anchors.verticalCenter: parent.verticalCenter; text: modelData.syntax; px: 12; color: T.muted }
                Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: modelData.key; px: 10; color: T.faint }
                MouseArea { id: recentHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: menu.apply(modelData) }
            }
        }
        Item { width: 1; height: T.s(6) }
    }
}
