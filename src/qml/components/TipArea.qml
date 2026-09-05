import QtQuick
import Brain

// Hover area that drives the shared tooltip and reports clicks. Coordinates
// are mapped to the window so the tooltip can flip near the edges.
MouseArea {
    id: area
    property var tip: null
    property bool pointer: true
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: pointer ? Qt.PointingHandCursor : Qt.ArrowCursor
    onPositionChanged: function(mouse) {
        if (!tip) return
        var p = area.mapToItem(null, mouse.x, mouse.y)
        T.showTip(tip, p.x, p.y)
    }
    onExited: T.hideTip()
    onPressed: T.hideTip()
}
