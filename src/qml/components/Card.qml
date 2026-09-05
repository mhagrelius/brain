import QtQuick
import Brain

Rectangle {
    default property alias content: inner.data
    property real radiusPx: 8
    color: T.card
    border.width: T.line
    border.color: T.border
    radius: T.s(radiusPx)
    clip: true
    implicitHeight: inner.childrenRect.y + inner.childrenRect.height + 2 * border.width
    Item {
        id: inner
        anchors.fill: parent
        anchors.margins: parent.border.width
    }
}
