import QtQuick
import Brain

Text {
    property real px: 12.5
    property int weight: Font.Normal
    font.family: T.mono
    font.pointSize: T.f(px)
    font.weight: weight
    color: T.text
    verticalAlignment: Text.AlignVCenter
}
