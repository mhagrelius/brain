import QtQuick
import Brain
import "../components"

// The right rail: THIS NOTE (a key/value table, then tag chips) and the
// backlinks. Tags and aliases are the editing surface for those frontmatter
// keys, since the editor never shows the block.
Item {
    id: rail
    Rectangle { width: T.line; height: parent.height; color: T.divider }
    VScroll {
        anchors.fill: parent
        anchors.leftMargin: T.line
        padding: T.s(16)
        Column {
            width: parent.width
            spacing: T.s(14)
            visible: App.hasNote

            Card {
                width: parent.width
                border.color: T.borderStrong
                Column {
                    x: T.s(14); y: T.s(14); width: parent.width - 2 * T.s(14)
                    Mono { text: "This note"; px: 10; color: T.faint; font.capitalization: Font.AllUppercase; font.letterSpacing: T.r(1.6) }
                    Item { width: 1; height: T.s(8) }
                    Column {
                        width: parent.width
                        Repeater {
                            model: [
                                {k: "folder", v: App.rail.folder || ""},
                                {k: "words", v: App.rail.words || ""},
                                {k: "created", v: App.rail.created || ""},
                                {k: "updated", v: App.rail.updated || ""},
                                {k: "aliases", v: App.rail.aliases || "", edit: "aliases"},
                                {k: "tags", v: App.rail.fmTags || "", edit: "tags"},
                                {k: "on disk", v: App.rail.onDisk || "", c: App.rail.onDiskTone}
                            ]
                            Item {
                                required property var modelData
                                required property int index
                                width: parent.width
                                height: T.s(26)
                                property bool editing: false
                                Rectangle { visible: index < 6; anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
                                Mono { anchors.verticalCenter: parent.verticalCenter; text: modelData.k; px: 11; color: T.muted }
                                Mono {
                                    visible: !editing
                                    anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                                    width: parent.width - T.s(70)
                                    horizontalAlignment: Text.AlignRight
                                    text: modelData.v.length ? modelData.v : (modelData.edit ? "—" : "")
                                    px: 12; elide: Text.ElideLeft
                                    color: modelData.c ? T.role(modelData.c) : (modelData.v.length ? T.text : T.dimmest)
                                }
                                TextInput {
                                    id: railInput
                                    visible: editing
                                    anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                                    width: parent.width - T.s(70)
                                    horizontalAlignment: Text.AlignRight
                                    font.family: T.mono; font.pointSize: T.f(12)
                                    color: T.text; selectionColor: T.activeFill; selectedTextColor: T.activeText
                                    clip: true
                                    function commit() { if (modelData.edit === "tags") App.setNoteTags(text); else App.setNoteAliases(text); editing = false; App.editorFocusRequested() }
                                    Keys.onReturnPressed: commit()
                                    Keys.onEscapePressed: { editing = false; App.editorFocusRequested() }
                                    onActiveFocusChanged: if (!activeFocus && editing) commit()
                                }
                                MouseArea {
                                    anchors.fill: parent
                                    visible: modelData.edit !== undefined && !editing && !App.reading
                                    cursorShape: Qt.IBeamCursor
                                    onClicked: { editing = true; railInput.text = modelData.v; railInput.forceActiveFocus(); railInput.cursorPosition = railInput.text.length }
                                }
                            }
                        }
                    }
                    Item { width: 1; height: T.s(12) }
                    Flow {
                        width: parent.width
                        spacing: T.s(6)
                        Repeater {
                            model: App.rail.tags || []
                            Rectangle {
                                required property string modelData
                                width: tagLabel.implicitWidth + 2 * T.s(6) + 2 * T.line
                                height: tagLabel.implicitHeight + 2 * T.s(1) + 2 * T.line
                                radius: T.s(4); color: "transparent"
                                border.width: T.line; border.color: T.tealBorder
                                Mono { id: tagLabel; anchors.centerIn: parent; text: "#" + modelData; px: 11; color: T.teal }
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.filterTag(modelData) }
                            }
                        }
                    }
                    Item { width: 1; height: T.s(14) }
                }
            }

            Card {
                width: parent.width
                border.color: T.borderStrong
                Column {
                    x: T.s(14); y: T.s(14); width: parent.width - 2 * T.s(14)
                    Item {
                        width: parent.width; height: blTitle.implicitHeight
                        Sans { id: blTitle; text: "Backlinks"; px: 13.5; weight: Font.Bold }
                        Mono { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; text: App.backlinks.length + (App.backlinks.length === 1 ? " note" : " notes"); px: 11; color: T.faint }
                    }
                    Item { width: 1; height: T.s(12) }
                    Mono { visible: App.backlinks.length === 0; text: "nothing links here yet"; px: 11; color: T.dimmest }
                    Column {
                        width: parent.width
                        spacing: T.s(10)
                        Repeater {
                            model: App.backlinks
                            Item {
                                required property var modelData
                                width: parent.width
                                height: Math.max(avatar.height, blText.height)
                                Rectangle {
                                    id: avatar
                                    width: T.s(26); height: T.s(26); radius: T.s(6)
                                    color: T.track; border.width: T.line; border.color: T.borderStrong
                                    Mono { anchors.centerIn: parent; text: modelData.initial; px: 11; color: T.meta }
                                }
                                Column {
                                    id: blText
                                    x: avatar.width + T.s(10); width: parent.width - x
                                    Sans { width: parent.width; text: modelData.title; px: 13; elide: Text.ElideRight }
                                    Mono { width: parent.width; text: modelData.context; px: 11; color: T.faint; wrapMode: Text.Wrap; lineHeight: 1.5; maximumLineCount: 3; elide: Text.ElideRight }
                                }
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.openNote(modelData.id) }
                            }
                        }
                    }
                    Item { width: 1; height: T.s(14) }
                }
            }
        }
    }
}
