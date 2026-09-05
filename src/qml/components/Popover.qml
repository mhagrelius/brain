import QtQuick
import Brain

// A summoned surface: 1px border, radius 8, window fill, soft shadow. The
// caller positions it; content goes in `body`.
Item {
    id: pop
    default property alias body: inner.data
    property real widthPx: 560
    property alias radiusPx: box.radius
    width: T.s(widthPx)
    height: inner.childrenRect.height + 2 * T.line
    // Shadow: three feathered rectangles read close enough to 0 18px 40px rgba(0,0,0,.5).
    Rectangle { anchors.fill: box; anchors.margins: -T.s(24); anchors.topMargin: -T.s(8); radius: T.s(30); color: Qt.rgba(0, 0, 0, 0.16) }
    Rectangle { anchors.fill: box; anchors.margins: -T.s(12); anchors.topMargin: 0; radius: T.s(18); color: Qt.rgba(0, 0, 0, 0.18) }
    Rectangle { anchors.fill: box; anchors.margins: -T.s(4); anchors.topMargin: T.s(4); radius: T.s(12); color: Qt.rgba(0, 0, 0, 0.22) }
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
