import QtQuick
import Brain

// 5px/11px padding, 6px radius, 1px border, hover brightens the border.
Rectangle {
    id: root
    property string text
    property bool mono: false
    property real px: 11.5
    property real padX: 11
    property real padY: 5
    property real radiusPx: 6
    property color fg: T.text2
    property color borderColor: T.border
    property color hoverBorder: T.faint
    property color fill: "transparent"
    property color hoverFill: fill
    signal clicked()
    width: label.implicitWidth + 2 * T.s(padX) + 2 * border.width
    height: label.implicitHeight + 2 * T.s(padY) + 2 * border.width
    radius: T.s(radiusPx)
    color: hit.containsMouse ? hoverFill : fill
    border.width: T.line
    border.color: hit.containsMouse ? hoverBorder : borderColor
    Text {
        id: label
        anchors.centerIn: parent
        text: root.text
        font.family: root.mono ? T.mono : T.sans
        font.pointSize: T.f(root.px)
        color: root.fg
    }
    MouseArea {
        id: hit
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }
    Accessible.role: Accessible.Button
    Accessible.name: root.text
}
