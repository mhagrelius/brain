import QtQuick
import Brain
import "../components"

// The sidebar: vault header, numbered nav, a separator, then the vault as a
// file tree (or the tag list, or the unfiled notes), and a footer rail.
// Every row sits on one 32px grid: an 11px leading slot, 14px, the label,
// a mono count on the right. Child notes indent by 37 (12 + 11 + 14).
Rectangle {
    id: side
    width: T.s(T.sidebarWidth)
    color: T.window

    property string dragId: ""
    property string dragFolder: ""
    property bool dragging: dragId.length > 0 || dragFolder.length > 0
    property string dropTarget: ""

    Column {
        id: top
        width: parent.width
        Item {
            width: parent.width
            height: T.s(16) + name.implicitHeight + T.s(4) + sub.implicitHeight + T.s(14)
            Sans { id: name; x: T.s(16); y: T.s(16); text: "Brain"; px: 16; weight: Font.Bold; font.letterSpacing: -T.r(0.16) }
            Mono { id: sub; x: T.s(16); y: name.y + name.implicitHeight + T.s(4); width: parent.width - T.s(32); text: App.vaultLabel; px: 11; color: T.faint; elide: Text.ElideMiddle }
        }
        Column {
            x: T.s(8); width: parent.width - T.s(16)
            Repeater {
                model: App.nav
                Rectangle {
                    required property var modelData
                    width: parent.width
                    height: T.s(T.rowHeight)
                    radius: T.s(6)
                    color: modelData.active ? T.activeFill : navHit.containsMouse ? T.card : "transparent"
                    Behavior on color { ColorAnimation { duration: 120 } }
                    Mono { x: T.s(12); width: T.s(11); anchors.verticalCenter: parent.verticalCenter; text: modelData.key; px: 11; color: modelData.active ? T.activeText : T.faint }
                    Sans { x: T.s(12) + T.s(11) + T.s(14); anchors.verticalCenter: parent.verticalCenter; text: modelData.label; px: 13.5; weight: modelData.active ? Font.Bold : Font.Normal; color: modelData.active ? T.text : T.text2 }
                    Mono { anchors.right: parent.right; anchors.rightMargin: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.count; px: 11; color: modelData.active ? T.activeText : T.role(modelData.tone) }
                    MouseArea { id: navHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.go(modelData.id) }
                }
            }
        }
        Item { width: 1; height: T.s(18) }
        Rectangle { width: parent.width; height: T.line; color: T.divider }
        Item {
            width: parent.width
            height: T.s(14) + label.implicitHeight + T.s(8)
            Mono { id: label; x: T.s(16); y: T.s(14); text: App.sidebarLabel; px: 10; color: T.faint; font.capitalization: App.sidebarLabel.startsWith("#") ? Font.MixedCase : Font.AllUppercase; font.letterSpacing: T.r(1.6) }
            Mono {
                anchors.right: parent.right; anchors.rightMargin: T.s(16); y: T.s(14)
                text: App.screen === "tags" && App.activeTag.length ? "clear ×" : App.screen === "notes" ? App.sortLabel : ""
                px: 10; color: sortHit.containsMouse ? T.text2 : T.dimmest
                MouseArea { id: sortHit; anchors.fill: parent; anchors.margins: -T.s(4); hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { if (App.screen === "tags") App.filterTag(""); else App.cycleSort() } }
            }
        }
    }

    VScroll {
        id: list
        anchors.top: top.bottom
        anchors.bottom: footer.top
        width: parent.width
        Column {
            x: T.s(8); width: parent.width - T.s(16)
            // Tags view: the tag tree, then the filtered notes beneath an active tag.
            Repeater {
                model: App.screen === "tags" && !App.activeTag.length ? App.tagRows : []
                Rectangle {
                    required property var modelData
                    width: parent.width
                    height: T.s(T.rowHeight)
                    radius: T.s(6)
                    color: modelData.active ? T.selection : tagHit.containsMouse ? T.card : "transparent"
                    Mono { x: T.s(12) + modelData.depth * T.s(14); width: T.s(11); anchors.verticalCenter: parent.verticalCenter; text: "#"; px: 11; color: T.teal }
                    Sans { x: T.s(12) + T.s(11) + T.s(14) + modelData.depth * T.s(14); anchors.verticalCenter: parent.verticalCenter; text: modelData.leaf; px: 13.5; color: modelData.active ? T.text : T.text2 }
                    Mono { anchors.right: parent.right; anchors.rightMargin: T.s(12); anchors.verticalCenter: parent.verticalCenter; text: modelData.count; px: 11; color: T.faint }
                    MouseArea { id: tagHit; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: App.filterTag(modelData.name) }
                }
            }
            Repeater {
                model: App.screen === "tags" && !App.activeTag.length ? [] : App.rows
                Item {
                    id: rowItem
                    required property var modelData
                    required property int index
                    readonly property bool isFolder: modelData.kind === "folder"
                    readonly property bool isTarget: side.dropTarget === (isFolder ? modelData.path : "")
                    width: parent.width
                    height: T.s(T.rowHeight) + (modelData.gap ? T.s(6) : 0) + (modelData.showExcerpt && modelData.excerpt.length ? T.s(14) : 0)

                    Rectangle {
                        id: rowBox
                        y: modelData.gap ? T.s(6) : 0
                        width: parent.width
                        height: parent.height - y
                        radius: T.s(6)
                        color: rowItem.isFolder && rowItem.isTarget && side.dragging ? T.activeFill
                             : modelData.selected ? T.selection
                             : hit.containsMouse ? T.card : "transparent"
                        Behavior on color { ColorAnimation { duration: 100 } }
                        Mono {
                            x: T.s(12) + modelData.depth * T.s(14); width: T.s(11); y: T.s(9)
                            visible: rowItem.isFolder
                            text: modelData.expanded ? "v" : ">"
                            px: 11; color: T.faint
                        }
                        Sans {
                            x: (rowItem.isFolder ? T.s(12) + T.s(11) + T.s(14) : T.s(37)) + modelData.depth * T.s(14)
                            y: T.s(7)
                            width: parent.width - x - metaText.width - T.s(24)
                            text: modelData.name
                            px: 13.5
                            weight: rowItem.isFolder ? Font.Bold : Font.Normal
                            color: modelData.selected ? T.text : T.text2
                            elide: Text.ElideRight
                        }
                        Mono {
                            visible: modelData.showExcerpt === true && (modelData.excerpt || "").length > 0
                            x: T.s(37) + modelData.depth * T.s(14); y: T.s(24)
                            width: parent.width - x - T.s(12)
                            text: modelData.excerpt || ""; px: 10; color: T.faint; elide: Text.ElideRight
                        }
                        Mono { id: metaText; anchors.right: parent.right; anchors.rightMargin: T.s(12); y: T.s(9); text: modelData.meta; px: 11; color: T.faint }
                        MouseArea {
                            id: hit
                            anchors.fill: parent
                            hoverEnabled: true
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            cursorShape: Qt.PointingHandCursor
                            property real pressX: 0
                            property real pressY: 0
                            property bool moved: false
                            onPressed: function(m) { pressX = m.x; pressY = m.y; moved = false }
                            onPositionChanged: function(m) {
                                if (!(m.buttons & Qt.LeftButton)) return
                                if (!moved && Math.abs(m.x - pressX) + Math.abs(m.y - pressY) > T.s(8)) {
                                    moved = true
                                    if (rowItem.isFolder) side.dragFolder = modelData.path; else side.dragId = modelData.id
                                }
                                if (moved) side.trackDrop(hit.mapToItem(list, m.x, m.y))
                            }
                            onReleased: function(m) {
                                if (moved) { side.finishDrop(); return }
                                if (m.button === Qt.RightButton) { menu.openFor(modelData, hit.mapToItem(side, m.x, m.y)); return }
                                if (rowItem.isFolder) App.toggleFolder(modelData.path); else App.openNote(modelData.id)
                            }
                            onCanceled: side.cancelDrop()
                        }
                        Keys.onPressed: function(e) { if (e.key === Qt.Key_F10 && (e.modifiers & Qt.ShiftModifier)) { menu.openFor(modelData, rowBox.mapToItem(side, T.s(40), rowBox.height)); e.accepted = true } }
                    }
                }
            }
            // The strip that names itself while a drag is in the air: drop here for the vault root.
            Rectangle {
                width: parent.width
                height: side.dragging ? T.s(T.rowHeight) : 0
                visible: side.dragging
                radius: T.s(6)
                color: side.dropTarget === "" && side.dragging && side.overRoot ? T.activeFill : "transparent"
                border.width: T.line; border.color: T.borderStrong
                Sans { anchors.centerIn: parent; text: "vault root"; px: 12; color: T.faint }
            }
        }
    }

    property bool overRoot: false
    function trackDrop(p) {
        dropTarget = ""
        overRoot = false
        var holder = list.contentItem.children[0]
        if (!holder) return
        var column = holder.children[0]
        if (!column) return
        var y = p.y + list.contentY - holder.y - column.y
        for (var j = 0; j < column.children.length; ++j) {
            var c = column.children[j]
            if (c.height <= 0 || !c.visible) continue
            if (y < c.y || y >= c.y + c.height) continue
            if (c.modelData === undefined) { overRoot = true; return }
            if (c.modelData.kind === "folder") dropTarget = c.modelData.path
            return
        }
    }
    function finishDrop() {
        if (dragId.length && (dropTarget.length || overRoot)) App.moveNote(dragId, dropTarget)
        else if (dragFolder.length && (dropTarget.length || overRoot)) App.moveFolder(dragFolder, dropTarget)
        cancelDrop()
    }
    function cancelDrop() { dragId = ""; dragFolder = ""; dropTarget = ""; overRoot = false }

    Rectangle {
        id: footer
        anchors.bottom: parent.bottom
        width: parent.width
        height: T.s(26)
        color: T.window
        Rectangle { width: parent.width; height: T.line; color: T.divider }
        Mono { x: T.s(16); anchors.verticalCenter: parent.verticalCenter; text: "Ctrl+K · insert Ctrl+Space"; px: 10; color: T.dimmest }
        Mono { anchors.right: parent.right; anchors.rightMargin: T.s(16); anchors.verticalCenter: parent.verticalCenter; text: "1-3"; px: 10; color: T.dimmest }
    }

    RowMenu { id: menu }
}
