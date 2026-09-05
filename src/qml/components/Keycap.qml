import QtQuick
import Brain

// A key hint: mono 10, 1px border, radius 4, padding 1 5.
Rectangle {
    property string text
    property real px: 10
    property color fg: T.dimmest
    width: label.implicitWidth + 2 * T.s(5) + 2 * T.line
    height: label.implicitHeight + 2 * T.s(1) + 2 * T.line
    radius: T.s(4)
    color: "transparent"
    border.width: T.line
    border.color: T.borderStrong
    Mono { id: label; anchors.centerIn: parent; text: parent.text; px: parent.px; color: parent.fg }
}
