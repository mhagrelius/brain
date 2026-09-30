import QtQuick
import Brain
import "../components"

// Unified search: titles (Ctrl+K) or the text of every note (Ctrl+Shift+F),
// the latter BM25 fused with vectors when a model is reachable. Tab flips
// between the two. The create affordance sits under the results.
Popover {
    id: panel
    widthPx: 660
    menuShadow: true   // the handoff shades the search panel like a menu
    property int pick: 0
    readonly property int count: App.hits.length
    Component.onCompleted: if (visible) { query.text = App.searchQuery; query.forceActiveFocus() }
    onVisibleChanged: if (visible) { query.text = App.searchQuery; pick = 0; query.forceActiveFocus() }
    Connections { target: App; function onChanged() { if (panel.pick > panel.count) panel.pick = 0 } }

    function open() { if (pick < count) App.openHit(App.hits[pick].id); else App.createFromSearch() }
    function lifted(row) {
        if (row.hlStart < 0) return row.detail
        var esc = function(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") }
        return esc(row.detail.substring(0, row.hlStart)) + "<font color='" + T.text2 + "'>" + esc(row.detail.substring(row.hlStart, row.hlEnd)) + "</font>" + esc(row.detail.substring(row.hlEnd))
    }

    Column {
        width: parent.width
        Item {
            width: parent.width; height: T.s(10) + crumbs.implicitHeight + T.s(8) + box.height + T.s(12)
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
            Row {
                id: crumbs
                x: T.s(14); y: T.s(10); spacing: T.s(6)
                Mono { text: "brain"; px: 10; color: T.faint; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(1.4) }
                Mono { text: "›"; px: 10; color: T.faint }
                Mono { text: App.searchMode === "titles" ? "titles" : "notes"; px: 10; color: T.accent; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(1.4) }
            }
            Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); y: T.s(10); text: App.searchRanking; px: 10; color: T.faint; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(1.4) }
            Rectangle {
                id: box
                x: T.s(14); y: crumbs.y + crumbs.implicitHeight + T.s(8); width: parent.width - 2 * T.s(14); height: T.s(32)
                radius: T.s(6); color: T.card; border.width: T.line; border.color: query.activeFocus ? T.accent : T.borderStrong
                Mono { x: T.s(10); anchors.verticalCenter: parent.verticalCenter; text: "/"; px: 12; color: T.accent }
                TextInput {
                    id: query
                    x: T.s(24); width: parent.width - x - T.s(10)
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: T.sans; font.pointSize: T.f(13.5)
                    color: T.text; selectionColor: T.activeFill; selectedTextColor: T.activeText
                    cursorDelegate: Rectangle { width: T.s(2); color: T.accent }
                    onTextEdited: { App.setSearchQuery(text); panel.pick = 0 }
                    Keys.onPressed: function(e) {
                        if (e.key === Qt.Key_Escape) { App.closeSearch(); e.accepted = true }
                        else if (e.key === Qt.Key_Tab) { App.searchToggleMode(); e.accepted = true }
                        else if (e.key === Qt.Key_Down) { panel.pick = Math.min(panel.count, panel.pick + 1); e.accepted = true }
                        else if (e.key === Qt.Key_Up) { panel.pick = Math.max(0, panel.pick - 1); e.accepted = true }
                        else if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) { panel.open(); e.accepted = true }
                    }
                    Sans { anchors.fill: parent; visible: !query.text.length; text: App.searchMode === "titles" ? "Go to a note by title" : "Search every note"; px: 13.5; color: T.muted }
                }
            }
        }
        Repeater {
            model: App.hits
            Rectangle {
                required property var modelData
                required property int index
                readonly property bool on: index === panel.pick
                width: parent.width; height: T.s(9) * 2 + Math.max(T.s(26), hitText.height)
                color: on ? T.selection : hitHit.containsMouse ? T.card : "transparent"
                Rectangle {
                    id: hitAvatar
                    x: T.s(14); y: T.s(9); width: T.s(26); height: T.s(26); radius: T.s(6)
                    color: T.track; border.width: T.line; border.color: T.borderStrong
                    Mono { anchors.centerIn: parent; text: modelData.initial; px: 11; color: T.meta }
                }
                Column {
                    id: hitText
                    x: T.s(14) + T.s(26) + T.s(10); y: T.s(9)
                    width: parent.width - x - T.s(70)
                    Sans { width: parent.width; text: modelData.title; px: 13.5; weight: on ? Font.Bold : Font.Normal; elide: Text.ElideRight }
                    Mono { width: parent.width; textFormat: Text.RichText; text: panel.lifted(modelData); px: 11; color: T.faint; elide: Text.ElideRight }
                }
                Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); y: T.s(9); text: modelData.score; px: 11; color: on ? T.positive : T.faint }
                MouseArea { id: hitHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.openHit(modelData.id) }
            }
        }
        Rectangle { width: parent.width; height: T.line; color: T.divider }
        Rectangle {
            visible: query.text.trim().length > 0
            readonly property bool on: panel.pick === panel.count
            width: parent.width; height: T.s(9) * 2 + T.s(26)
            color: on ? T.selection : createHit.containsMouse ? T.card : "transparent"
            Rectangle {
                x: T.s(14); y: T.s(9); width: T.s(26); height: T.s(26); radius: T.s(6)
                color: T.positiveBg; border.width: T.line; border.color: T.positiveDim
                Mono { anchors.centerIn: parent; text: "+"; px: 13; color: T.positive }
            }
            Sans { x: T.s(50); anchors.verticalCenter: parent.verticalCenter; width: parent.width - x - T.s(120); text: "New note “" + query.text.trim() + "”"; px: 13.5; color: T.positive; elide: Text.ElideRight }
            Mono { anchors.right: parent.right; anchors.rightMargin: T.s(14); anchors.verticalCenter: parent.verticalCenter; text: App.createDestination; px: 11; color: T.faint }
            MouseArea { id: createHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.createFromSearch() }
        }
        Item {
            width: parent.width; height: T.s(28)
            Rectangle { width: parent.width; height: T.line; color: T.divider }
            Row {
                x: T.s(14); anchors.verticalCenter: parent.verticalCenter; spacing: T.s(16)
                Mono { text: "↑↓ move"; px: 10; color: T.dimmest }
                Mono { text: "return open"; px: 10; color: T.dimmest }
                Mono { text: "tab titles / text"; px: 10; color: T.dimmest }
                Mono { text: "esc close"; px: 10; color: T.dimmest }
            }
        }
    }
}
