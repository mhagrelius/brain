import QtQuick
import Brain

// Row surface with hover fill, optional bottom divider and tooltip.
Rectangle {
    id: root
    property var tip: null
    property bool clickable: true
    property bool hoverable: clickable   // hover fill without the hand cursor
    property color hoverColor: T.hover
    property color dividerColor: T.divider
    property bool showDivider: true
    property bool selected: false
    property color selectedColor: T.selection
    signal clicked()
    readonly property alias hovered: area.containsMouse
    color: selected ? selectedColor : (area.containsMouse && hoverable ? hoverColor : "transparent")
    Rectangle {
        visible: root.showDivider
        anchors.bottom: parent.bottom
        width: parent.width
        height: T.line
        color: root.dividerColor
    }
    TipArea {
        id: area
        tip: root.tip
        pointer: root.clickable
        onClicked: root.clicked()
    }
}
