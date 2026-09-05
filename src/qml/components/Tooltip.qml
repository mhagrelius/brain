import QtQuick
import Brain

// The single cursor-following tooltip. Offset +16/+18 from the cursor and
// flipped when it would leave the window.
Rectangle {
    id: root
    property var payload: T.tip
    property real cursorX: T.tipX
    property real cursorY: T.tipY
    property real hostWidth: parent ? parent.width : 1360
    property real hostHeight: parent ? parent.height : 900
    visible: payload !== null && payload !== undefined
    width: Math.max(T.s(176), Math.min(T.s(268), body.implicitWidth + 2 * T.s(11)))
    height: body.implicitHeight + T.s(9) + T.s(10)
    x: cursorX > hostWidth - T.s(200) ? cursorX - T.s(288) : cursorX + T.s(16)
    y: cursorY > hostHeight - T.s(200) ? cursorY - T.s(172) : cursorY + T.s(18)
    z: 60
    color: Qt.rgba(T.tooltipBg.r, T.tooltipBg.g, T.tooltipBg.b, 0.98)
    border.width: T.line
    border.color: T.borderStrong
    radius: T.s(7)
    Column {
        id: body
        x: T.s(11)
        y: T.s(9)
        width: parent.width - 2 * T.s(11)
        Sans { width: parent.width; text: payload ? payload.title : ""; px: 11.5; weight: Font.DemiBold; elide: Text.ElideRight }
        Item { width: 1; height: T.s(2) }
        Mono { width: parent.width; text: payload ? payload.sub : ""; px: 9.5; color: T.muted; elide: Text.ElideRight; visible: text.length > 0 }
        Item { width: 1; height: T.s(8) }
        Column {
            width: parent.width
            spacing: T.s(4)
            Repeater {
                model: payload ? payload.rows : []
                Item {
                    required property var modelData
                    width: parent.width
                    height: Math.max(k.implicitHeight, v.implicitHeight)
                    Mono { id: k; anchors.left: parent.left; anchors.right: v.left; anchors.rightMargin: T.s(16); text: modelData.k; px: 10.5; color: T.muted; elide: Text.ElideRight }
                    Mono { id: v; anchors.right: parent.right; text: modelData.v; px: 10.5; color: modelData.c }
                }
            }
        }
        Item { width: 1; height: payload && payload.hint ? T.s(9) : 0 }
        Rectangle { width: parent.width; height: T.line; color: T.border; visible: payload && payload.hint ? true : false }
        Item { width: 1; height: payload && payload.hint ? T.s(7) : 0 }
        Mono { width: parent.width; text: payload && payload.hint ? payload.hint : ""; px: 9.5; color: T.faint; visible: text.length > 0; wrapMode: Text.WordWrap }
    }
}
