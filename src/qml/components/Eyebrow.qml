import QtQuick
import Brain

// Uppercase 10px label with 0.09em tracking.
Text {
    property real px: 10
    font.family: T.sans
    font.pointSize: T.f(px)
    font.capitalization: Font.AllUppercase
    font.letterSpacing: T.r(px * 0.09)
    color: T.muted
}
