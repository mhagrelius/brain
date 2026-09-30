import QtQuick
import Brain

// A summoned surface: 1px border, radius 8, window fill, soft shadow. The
// caller positions it; content goes in `body`.
Item {
    id: pop
    default property alias body: inner.data
    property real widthPx: 560
    property alias radiusPx: box.radius
    // Panels take the popover shadow, menus and search the heavier menu one.
    property bool menuShadow: false
    width: T.s(widthPx)
    height: inner.childrenRect.height + 2 * T.line
    DropShadow { source: box; anchors.fill: box; menu: pop.menuShadow }
    Rectangle {
        id: box
        anchors.fill: parent
        radius: T.s(8)
        color: T.window
        border.width: T.line
        border.color: T.border
        clip: true
        Item { id: inner; anchors.fill: parent; anchors.margins: T.line }
    }
    MouseArea { anchors.fill: parent; z: -1; onClicked: function(m) { m.accepted = true } }
}
