import QtQuick
import Brain

// A symbolic icon in a palette colour, from src/icons/<name>.svg via the icon
// provider. sourceSize at 2× keeps it crisp on a scaled monitor.
Image {
    property string name: "folder"
    property color tint: T.muted
    property real px: 15
    width: T.s(px)
    height: T.s(px)
    sourceSize: Qt.size(T.s(px) * 2, T.s(px) * 2)
    source: name.length ? "image://icon/" + name + "/" + Qt.rgba(tint.r, tint.g, tint.b, 1).toString() : ""
    smooth: true
    mipmap: true
    fillMode: Image.PreserveAspectFit
}
