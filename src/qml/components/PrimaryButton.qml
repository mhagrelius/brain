import QtQuick
import Brain

// Accent fill, dark text, hover brightness(1.08).
Rectangle {
    id: root
    property string text
    property real px: 12
    property real padX: 13
    property real padY: 6
    property color fill: T.accent
    property color fg: T.accentText
    property real hoverGain: 1.08
    signal clicked()
    width: label.implicitWidth + 2 * T.s(padX)
    height: label.implicitHeight + 2 * T.s(padY)
    radius: T.s(6)
    color: !enabled ? T.border : hit.containsMouse ? T.brighten(fill, hoverGain) : fill
    Sans {
        id: label
        anchors.centerIn: parent
        text: root.text
        px: root.px
        weight: Font.DemiBold
        color: root.enabled ? root.fg : T.faint
    }
    MouseArea {
        id: hit
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ForbiddenCursor
        onClicked: if (root.enabled) root.clicked()
    }
    Accessible.role: Accessible.Button
    Accessible.name: root.text
}
