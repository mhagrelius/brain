import QtQuick
import QtQuick.Window
import Brain
import "../components"

// The content column: header (title, subtitle, search, the one amber
// action) and the editor. The editor is one TextEdit over App.editor's
// display text; App.editor hides syntax outside the caret's construct and
// styles the rest through the document.
Item {
    id: pane
    property int selStart: 0
    property int selEnd: 0

    function focusSearch() { searchInput.forceActiveFocus(); searchInput.selectAll() }
    function focusEditor() { if (!App.reading) { textEdit.forceActiveFocus() } }
    function rememberSelection() { selStart = textEdit.selectionStart; selEnd = textEdit.selectionEnd; App.rememberSelection(selStart, selEnd) }
    function format(label) { if (!App.hasNote || App.reading) return; App.editor.applyFormat(label, textEdit.selectionStart, textEdit.selectionEnd); textEdit.forceActiveFocus() }

    // ---- header ---------------------------------------------------------------
    Rectangle {
        id: header
        width: parent.width
        height: T.s(14) + titleText.implicitHeight + T.s(3) + subText.implicitHeight + T.s(14)
        color: T.window
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: T.line; color: T.divider }
        Sans { id: titleText; x: T.s(20); y: T.s(14); width: parent.width - T.s(40) - right.width - T.s(16); text: App.noteTitle; px: 15; weight: Font.Bold; elide: Text.ElideRight }
        Mono { id: subText; x: T.s(20); y: titleText.y + titleText.implicitHeight + T.s(3); width: titleText.width; text: App.noteSubtitle; px: 11; color: T.faint; elide: Text.ElideRight }
        Row {
            id: right
            anchors.right: parent.right; anchors.rightMargin: T.s(20)
            anchors.verticalCenter: parent.verticalCenter
            spacing: T.s(10)
            // Search: 250×32, ⌕ glyph, placeholder, a "/" keycap. Filters the tree as you type.
            Rectangle {
                width: T.s(250); height: T.s(32)
                radius: T.s(6); color: T.card
                border.width: T.line; border.color: searchInput.activeFocus ? T.accent : T.borderStrong
                Mono { id: glyph; x: T.s(10); anchors.verticalCenter: parent.verticalCenter; text: "⌕"; px: 11; color: T.dimmest }
                TextInput {
                    id: searchInput
                    x: T.s(28); width: parent.width - x - keycap.width - T.s(18)
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: T.sans; font.pointSize: T.f(13)
                    color: T.text; selectionColor: T.activeFill; selectedTextColor: T.activeText
                    clip: true
                    onTextEdited: App.setQuery(text)
                    Keys.onEscapePressed: { text = ""; App.setQuery(""); pane.focusEditor() }
                    Keys.onReturnPressed: { if (App.rows.length > 0 && App.rows[0].kind === "note") App.openNote(App.rows[0].id); pane.focusEditor() }
                    Sans { anchors.fill: parent; text: "Search notes"; px: 13; color: T.muted; visible: !searchInput.text.length }
                }
                Keycap { id: keycap; anchors.right: parent.right; anchors.rightMargin: T.s(8); anchors.verticalCenter: parent.verticalCenter; text: "/" }
            }
            PrimaryButton { text: "+ New note"; px: 13; padX: 14; padY: 0; height: T.s(32); anchors.verticalCenter: parent.verticalCenter; onClicked: App.newNote() }
        }
    }

    // ---- empty state ----------------------------------------------------------
    Column {
        anchors.centerIn: editorArea
        visible: !App.hasNote
        spacing: T.s(8)
        Sans { anchors.horizontalCenter: parent.horizontalCenter; text: "No note open"; px: 14; color: T.muted }
        Mono { anchors.horizontalCenter: parent.horizontalCenter; text: "pick one on the left · Ctrl+N writes a new one · Ctrl+K finds one"; px: 11; color: T.dimmest }
    }

    // ---- editor ---------------------------------------------------------------
    Flickable {
        id: editorArea
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        visible: App.hasNote
        clip: true
        contentWidth: width
        contentHeight: textEdit.y + textEdit.contentHeight + T.s(120)
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        readonly property real padX: T.s(28)
        readonly property real bodyWidth: Math.min(width - 2 * padX, T.s(660))

        function ensureVisible(r) {
            if (contentY >= r.y) contentY = r.y - T.s(20)
            else if (contentY + height <= r.y + r.height) contentY = r.y + r.height - height + T.s(40)
        }

        // A click below the last line puts the caret at the end.
        MouseArea {
            x: textEdit.x; width: textEdit.width
            y: textEdit.y + textEdit.contentHeight; height: Math.max(0, editorArea.contentHeight - y)
            cursorShape: Qt.IBeamCursor
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: function(m) {
                if (App.reading) return
                textEdit.forceActiveFocus()
                textEdit.cursorPosition = textEdit.length
                if (m.button === Qt.RightButton) { pane.rememberSelection(); App.openInsert() }
            }
        }

        // Decorations behind the text: quote bars, table and code boxes.
        Item {
            x: editorArea.padX; y: textEdit.y
            width: editorArea.bodyWidth
            Repeater {
                model: App.editor.decorations
                Item {
                    required property var modelData
                    readonly property bool ready: modelData.to <= textEdit.length
                    readonly property rect from: { var tie = textEdit.length + textEdit.contentHeight + textEdit.width; return ready ? textEdit.positionToRectangle(modelData.from) : Qt.rect(0, 0, 0, 0) }
                    readonly property rect to: { var tie = textEdit.length + textEdit.contentHeight + textEdit.width; return ready ? textEdit.positionToRectangle(modelData.to) : Qt.rect(0, 0, 0, 0) }
                    visible: ready
                    y: from.y - T.s(2)
                    height: to.y + to.height - from.y + T.s(4)
                    width: parent.width
                    Rectangle { visible: modelData.kind === "quote"; x: 0; width: T.s(2); height: parent.height; color: T.activeFill }
                    Rectangle { visible: modelData.kind === "table" || modelData.kind === "code"; anchors.fill: parent; radius: T.s(6); color: T.card; border.width: T.line; border.color: T.divider }
                }
            }
        }

        TextEdit {
            id: textEdit
            x: editorArea.padX
            y: T.s(26)
            width: editorArea.bodyWidth
            textFormat: TextEdit.PlainText
            wrapMode: TextEdit.Wrap
            font.family: T.sans
            font.pointSize: T.f(14.5)
            color: T.text
            selectionColor: T.alpha(T.accent, 0.28)
            selectedTextColor: T.text
            readOnly: App.reading
            cursorVisible: !App.reading && activeFocus
            cursorDelegate: Rectangle { width: T.s(2); color: T.accent; visible: textEdit.cursorVisible && !App.reading }
            persistentSelection: true
            selectByMouse: true
            tabStopDistance: T.s(28)
            Component.onCompleted: { App.editor.document = textEdit.textDocument; App.editor.textEdit = textEdit; App.editor.contentWidth = width }
            onWidthChanged: App.editor.contentWidth = width
            onCursorPositionChanged: App.editor.cursorMoved(cursorPosition)
            onActiveFocusChanged: App.editor.focused = activeFocus
            onCursorRectangleChanged: if (activeFocus) editorArea.ensureVisible(cursorRectangle)
            Keys.onPressed: function(e) {
                if (App.editor.completing && completion.count > 0 && !(e.modifiers & Qt.ControlModifier)) {
                    if (e.key === Qt.Key_Down) { completion.pick = Math.min(completion.count - 1, completion.pick + 1); e.accepted = true; return }
                    if (e.key === Qt.Key_Up) { completion.pick = Math.max(0, completion.pick - 1); e.accepted = true; return }
                    if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter || e.key === Qt.Key_Tab) { App.editor.acceptCompletion(completion.candidates[completion.pick]); e.accepted = true; return }
                    if (e.key === Qt.Key_Escape) { App.editor.dismissCompletion(); e.accepted = true; return }
                }
                if (e.key === Qt.Key_Escape) { pane.forceActiveFocus(); Window.window.contentItem.forceActiveFocus(); e.accepted = true; return }
                if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter) && !(e.modifiers & Qt.ShiftModifier)) {
                    if (e.modifiers & Qt.ControlModifier) { var t = App.editor.linkAt(cursorPosition); if (t.length) { App.followLink(t); e.accepted = true; return } }
                    if (App.editor.continueList(cursorPosition)) { e.accepted = true; return }
                }
                if (e.key === Qt.Key_Backspace && !(e.modifiers & Qt.ControlModifier) && textEdit.selectionStart === textEdit.selectionEnd) { if (App.editor.backspaceBullet(cursorPosition)) { e.accepted = true; return } }
                if (e.key === Qt.Key_Tab) { App.editor.insertAtCaret("  ", cursorPosition); e.accepted = true; return }
                if (e.key === Qt.Key_Z && (e.modifiers & Qt.ControlModifier)) { if (e.modifiers & Qt.ShiftModifier) App.editor.redo(); else App.editor.undo(); e.accepted = true; return }
                if (e.key === Qt.Key_Y && (e.modifiers & Qt.ControlModifier)) { App.editor.redo(); e.accepted = true; return }
                if (e.key === Qt.Key_V && (e.modifiers & Qt.ControlModifier)) { if (App.pasteImage()) e.accepted = true }
            }
        }

        // Embedded images, drawn under the line that names them.
        Repeater {
            model: App.editor.embeds
            Item {
                required property var modelData
                readonly property bool ready: modelData.pos <= textEdit.length
                readonly property rect at: { var tie = textEdit.length + textEdit.contentHeight + textEdit.width; return ready ? textEdit.positionToRectangle(modelData.pos) : Qt.rect(0, 0, 0, 0) }
                visible: ready
                x: editorArea.padX
                y: textEdit.y + at.y + at.height + T.s(6)
                width: modelData.width
                height: modelData.height
                Rectangle { anchors.fill: parent; radius: T.s(8); color: T.card; border.width: T.line; border.color: T.borderStrong }
                Image { anchors.fill: parent; anchors.margins: T.line; source: "file://" + modelData.path; fillMode: Image.PreserveAspectFit; asynchronous: true; smooth: true }
                Mono { anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: T.s(8); text: "attachment · " + modelData.name; px: 10; color: T.faint }
            }
        }

        // Ctrl+Click follows a link, toggles a task; a plain click does in reading mode.
        // Right-click opens the insert menu at the caret, keeping a selection.
        MouseArea {
            x: textEdit.x; y: textEdit.y; width: textEdit.width; height: textEdit.contentHeight
            hoverEnabled: true
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            propagateComposedEvents: true
            cursorShape: hoverLink ? Qt.PointingHandCursor : Qt.IBeamCursor
            property bool hoverLink: false
            onPositionChanged: function(m) {
                var p = textEdit.positionAt(m.x, m.y)
                var linky = App.editor.linkAt(p).length > 0 || App.editor.urlAt(p).length > 0 || App.editor.tagAt(p).length > 0
                hoverLink = linky && (App.reading || (m.modifiers & Qt.ControlModifier))
            }
            onPressed: function(m) {
                if (m.button === Qt.RightButton) {
                    if (App.reading) return
                    var at = textEdit.positionAt(m.x, m.y)
                    if (textEdit.selectionStart === textEdit.selectionEnd || at < textEdit.selectionStart || at > textEdit.selectionEnd) textEdit.cursorPosition = at
                    textEdit.forceActiveFocus()
                    pane.rememberSelection()
                    App.openInsert()
                    return
                }
                var wants = App.reading || (m.modifiers & Qt.ControlModifier)
                if (!wants) { m.accepted = false; return }
                var p = textEdit.positionAt(m.x, m.y)
                var link = App.editor.linkAt(p)
                if (link.length) { App.followLink(link); return }
                var url = App.editor.urlAt(p)
                if (url.length) { App.openUrl(url); return }
                var tag = App.editor.tagAt(p)
                if (tag.length) { App.filterTag(tag); return }
                if (App.editor.toggleTaskAt(p)) return
                m.accepted = false
            }
        }

        // [[ completion: the same candidates capture uses.
        Rectangle {
            id: completion
            property var candidates: App.editor.completing ? App.linkCandidates(App.editor.completionQuery) : []
            property int count: candidates.length
            property int pick: 0
            onCandidatesChanged: pick = 0
            readonly property rect at: { var tie = textEdit.length + textEdit.contentHeight; return App.editor.completionAnchor <= textEdit.length ? textEdit.positionToRectangle(App.editor.completionAnchor) : Qt.rect(0, 0, 0, 0) }
            visible: App.editor.completing && count > 0
            x: Math.min(textEdit.x + at.x, editorArea.width - width - T.s(12))
            y: textEdit.y + at.y + at.height + T.s(4)
            width: T.s(300)
            height: compCol.height + 2 * T.s(4) + 2 * T.line
            radius: T.s(6); color: T.card; border.width: T.line; border.color: T.border
            z: 5
            Column {
                id: compCol
                x: T.s(4); y: T.s(4); width: parent.width - T.s(8)
                Repeater {
                    model: completion.candidates
                    Rectangle {
                        required property string modelData
                        required property int index
                        width: parent.width; height: T.s(30); radius: T.s(4)
                        color: index === completion.pick ? T.selection : "transparent"
                        Sans { x: T.s(10); anchors.verticalCenter: parent.verticalCenter; text: modelData; px: 13 }
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: App.editor.acceptCompletion(modelData) }
                    }
                }
            }
        }

        DropArea {
            anchors.fill: parent
            onDropped: function(drop) { if (drop.hasUrls) { var list = []; for (var i = 0; i < drop.urls.length; ++i) list.push(String(drop.urls[i])); App.attachUrls(list); drop.accept() } }
        }
    }
}
