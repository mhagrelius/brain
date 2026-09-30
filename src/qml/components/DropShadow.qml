import QtQuick
import QtQuick.Effects
import Brain

// The handoff's drop shadow, rendered by the platform effect instead of
// stacked hard rectangles: popover 0 18px 40px rgba(0,0,0,.5), menu
// 0 22px 50px rgba(0,0,0,.55). Colour is the fixed `shadow` role; the
// geometry and alpha are the handoff's literal values. Declare it before
// the panel it shades; the panel stays visible and keeps taking input.
MultiEffect {
    id: shadow

    property bool menu: false
    readonly property real blurPx: menu ? 50 : 40
    readonly property real offsetPx: menu ? 22 : 18
    readonly property real alpha: menu ? 0.55 : 0.5

    // autoPadding bleeds blurMax past the item on every side; the vertical
    // offset shifts coverage down by moving paddingRect the other way.
    autoPaddingEnabled: true
    paddingRect: Qt.rect(0, -T.r(offsetPx), 0, T.r(offsetPx))
    shadowEnabled: true
    shadowBlur: 1.0
    blurMax: Math.round(T.r(blurPx))
    shadowColor: T.shadow
    shadowOpacity: alpha
    shadowVerticalOffset: T.r(offsetPx)
}
