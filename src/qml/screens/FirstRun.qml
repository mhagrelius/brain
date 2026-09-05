import QtQuick
import QtQuick.Dialogs
import Brain
import "../components"

// The first launch: a folder to keep notes in. An existing folder of
// Markdown files works as it is.
Item {
    Card {
        anchors.centerIn: parent
        width: T.s(520)
        border.color: T.borderStrong
        Column {
            x: T.s(24); y: T.s(22); width: parent.width - 2 * T.s(24)
            spacing: T.s(10)
            Sans { text: "Where do the notes live?"; px: 16; weight: Font.Bold }
            Mono { width: parent.width; text: "A vault is a folder of .md files. Point Brain at one you already have, or let it make this one."; px: 11.5; color: T.meta; wrapMode: Text.Wrap; lineHeight: 1.6 }
            Item { width: 1; height: T.s(4) }
            Field {
                id: path
                width: parent.width
                mono: true
                px: 12.5
                text: App.suggestedVault()
                onAccepted: App.chooseVault(text)
                background: Rectangle { color: T.card; radius: T.s(6); border.width: T.line; border.color: path.activeFocus ? T.accent : T.borderStrong }
            }
            Row {
                spacing: T.s(8)
                PrimaryButton { text: "Use this folder"; px: 13; padX: 14; padY: 0; height: T.s(32); onClicked: App.chooseVault(path.text) }
                SecondaryButton { text: "Browse…"; px: 13; padX: 14; padY: 0; height: T.s(32); fill: T.card; hoverFill: T.track; borderColor: T.borderStrong; fg: T.text2; onClicked: chooser.open() }
            }
            Item { width: 1; height: T.s(8) }
        }
    }
    FolderDialog { id: chooser; onAccepted: App.chooseVault(String(selectedFolder)) }
}
