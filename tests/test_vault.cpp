#include <QtTest>

#include "vault.h"

using namespace brain;

class TestVault : public QObject {
    Q_OBJECT
private slots:
    void scanSkipsDotfilesAttachmentsAndNonNotes() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        QDir().mkpath(dir.path() + "/.brain");
        QDir().mkpath(dir.path() + "/attachments");
        QDir().mkpath(dir.path() + "/Meetings");
        auto write = [&](const QString &rel, const QByteArray &bytes) { QFile f(dir.path() + "/" + rel); f.open(QIODevice::WriteOnly); f.write(bytes); };
        write(".brain/x.md", "{}");
        write("attachments/pic.png", "\x89PNG");
        write("Meetings/standup.md", "# Standup\n");
        write("note.md", "body");
        write("readme.txt", "no");
        QVector<VaultError> problems;
        const QVector<Note> notes = vault.scan(&problems);
        QVERIFY(problems.isEmpty());
        QCOMPARE(notes.size(), 2);
        QCOMPARE(notes[0].id.str(), QStringLiteral("Meetings/standup.md"));
        QCOMPARE(notes[1].id.str(), QStringLiteral("note.md"));
        QVERIFY(!vault.idOf(dir.path() + "/.brain/x.md"));
        QCOMPARE(vault.folders(), QStringList{"Meetings"});
    }
    void writeIsAtomicAndCreatesFolders() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        const Note note = Note::fromText(NoteId::fromRelative("deep/er/n.md"), "---\ntags: [a]\n---\nbody\n");
        QVERIFY(vault.write(note));
        QCOMPARE(vault.read(note.id)->toText(), note.toText());
        QVERIFY(QDir(dir.path() + "/deep/er").entryList(QDir::Files).size() == 1);   // no .tmp left behind
    }
    void createRefusesToClobber() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        QVERIFY(vault.create(NoteId::fromRelative("a.md"), "one"));
        VaultError error;
        QVERIFY(!vault.create(NoteId::fromRelative("a.md"), "two", &error));
        QVERIFY(error.message.contains("exists"));
        QCOMPARE(vault.read(NoteId::fromRelative("a.md"))->body, QStringLiteral("one"));
    }
    void renameAndDelete() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        vault.create(NoteId::fromRelative("a.md"), "x");
        vault.create(NoteId::fromRelative("b.md"), "y");
        QVERIFY(!vault.rename(NoteId::fromRelative("a.md"), NoteId::fromRelative("b.md")));
        QVERIFY(vault.rename(NoteId::fromRelative("a.md"), NoteId::fromRelative("f/c.md")));
        QVERIFY(vault.read(NoteId::fromRelative("f/c.md")));
        QVERIFY(vault.remove(NoteId::fromRelative("b.md")));
        QVERIFY(!vault.read(NoteId::fromRelative("b.md")));
    }
    void foldersOnlyGoWhenEmpty() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        QVERIFY(vault.createFolder("a/b"));
        vault.create(NoteId::fromRelative("a/b/n.md"), "");
        QVERIFY(!vault.deleteFolder("a/b"));
        vault.remove(NoteId::fromRelative("a/b/n.md"));
        QVERIFY(vault.deleteFolder("a/b"));
        QVERIFY(!vault.moveFolder("a", "a/x"));
        QVERIFY(vault.moveFolder("a", "z"));
        QCOMPARE(vault.folders(), QStringList{"z"});
    }
    void attachmentsDeduplicateBySameBytes() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        QCOMPARE(*vault.addAttachmentBytes("pic.png", "AAA"), QStringLiteral("pic.png"));
        QCOMPARE(*vault.addAttachmentBytes("pic.png", "AAA"), QStringLiteral("pic.png"));
        QCOMPARE(*vault.addAttachmentBytes("pic.png", "BBB"), QStringLiteral("pic-1.png"));
        QVERIFY(QFile::exists(vault.attachmentPath("pic-1.png")));
    }
};

QTEST_APPLESS_MAIN(TestVault)
#include "test_vault.moc"
