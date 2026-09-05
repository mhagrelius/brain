#include <QtTest>

#include "notebook.h"

using namespace brain;

namespace {
NoteId id(const QString &p) { return NoteId::fromRelative(p); }
struct Fixture {
    QTemporaryDir dir;
    Notebook notebook;
    Fixture() {
        notebook.loadConfig(dir.path() + "/config/brain/config.json");
        notebook.setVault(dir.path() + "/vault");
    }
    void write(const QString &rel, const QString &text) { QVERIFY(notebook.vault()->writeText(id(rel), text)); notebook.rescan(); }
};
}

class TestNotebook : public QObject {
    Q_OBJECT
private slots:
    void createOpensAndSavesThroughTheTick() {
        Fixture f;
        const auto made = f.notebook.createNoteIn("", "Rust ownership");
        QVERIFY(made);
        QCOMPARE(f.notebook.openNoteId()->str(), QStringLiteral("Rust ownership.md"));
        QCOMPARE(f.notebook.saveNow(), Notebook::Saved::Clean);
        f.notebook.flushBody("# Rust ownership\n\nMoves.\n");
        QVERIFY(f.notebook.isDirty());
        QCOMPARE(f.notebook.saveNow(), Notebook::Saved::Written);
        QCOMPARE(f.notebook.vault()->read(*made)->body, QStringLiteral("# Rust ownership\n\nMoves.\n"));
        QCOMPARE(f.notebook.index().excerpt(*made), QStringLiteral("Moves."));
        // A second note with the same title gets a suffix.
        QCOMPARE(f.notebook.createNoteIn("", "Rust ownership")->str(), QStringLiteral("Rust ownership 2.md"));
    }
    void renameRepointsInboundLinks() {
        Fixture f;
        f.write("A.md", "see [[B|it]] and [[B]]\n");
        f.write("B.md", "text\n");
        f.write("C.md", "no links\n");
        QVERIFY(f.notebook.loadNote(id("B.md")));
        const Notebook::Renamed r = f.notebook.renameNote("D");
        QCOMPARE(r.kind, Notebook::Renamed::Done);
        QCOMPARE(r.links, 1);
        QCOMPARE(f.notebook.vault()->read(id("A.md"))->body, QStringLiteral("see [[D|it]] and [[D]]\n"));
        QCOMPARE(f.notebook.openNoteId()->str(), QStringLiteral("D.md"));
        QCOMPARE(f.notebook.backlinksOfOpenNote().size(), 2);
    }
    void moveKeepsLinksResolving() {
        Fixture f;
        f.write("A.md", "[[B]]\n");
        f.write("B.md", "text\n");
        f.notebook.createFolder("", "Folder");
        const Notebook::Moved m = f.notebook.moveNote(id("B.md"), "Folder");
        QCOMPARE(m.kind, Notebook::Moved::Done);
        QCOMPARE(f.notebook.resolveLink("B").note.str(), QStringLiteral("Folder/B.md"));
        QVERIFY(f.notebook.expanded().contains("Folder"));
        QVERIFY(f.notebook.relocateFolder("Folder", "Other/Deep"));
        QCOMPARE(f.notebook.resolveLink("B").note.str(), QStringLiteral("Other/Deep/B.md"));
        QVERIFY(!f.notebook.deleteFolder("Other"));   // not empty
    }
    void deleteDropsThePendingWriteFirst() {
        Fixture f;
        f.write("A.md", "text\n");
        f.notebook.loadNote(id("A.md"));
        f.notebook.flushBody("edited");
        QVERIFY(f.notebook.deleteOpenNote());
        QVERIFY(!f.notebook.isDirty());
        QVERIFY(!f.notebook.vault()->read(id("A.md")));
        QVERIFY(!f.notebook.openNoteId());
    }
    void anExternalChangeIsAChoiceOnlyWhenDirty() {
        Fixture f;
        f.write("A.md", "v1\n");
        f.notebook.loadNote(id("A.md"));
        QCOMPARE(f.notebook.absorbExternalChanges().kind, External::Quiet);   // our own write is not a change
        f.notebook.vault()->writeText(id("A.md"), "v2\n");
        const External e = f.notebook.absorbExternalChanges();
        QCOMPARE(e.kind, External::Reloaded);
        QCOMPARE(f.notebook.openNote()->body, QStringLiteral("v2\n"));
        f.notebook.flushBody("mine\n");
        f.notebook.vault()->writeText(id("A.md"), "v3\n");
        QCOMPARE(f.notebook.absorbExternalChanges().kind, External::Diverged);
        QCOMPARE(f.notebook.alert().kind, Alert::Diverged);
        QVERIFY(f.notebook.takeDiskVersion());
        QCOMPARE(f.notebook.openNote()->body, QStringLiteral("v3\n"));
        QCOMPARE(f.notebook.alert().kind, Alert::None);
        f.notebook.vault()->remove(id("A.md"));
        QCOMPARE(f.notebook.absorbExternalChanges().kind, External::Vanished);
        QVERIFY(f.notebook.restoreOpenNote());
        QVERIFY(f.notebook.vault()->read(id("A.md")));
    }
    void theSidebarSearchesTitlesThenText() {
        Fixture f;
        f.write("Bread.md", "flour and water\n");
        f.write("Notes.md", "about bread\n");
        f.write("Other.md", "nothing\n");
        f.notebook.setQuery("bread");
        const auto results = f.notebook.searchResults();
        QCOMPARE(results.size(), 2);
        QCOMPARE(results[0].first.title(), QStringLiteral("Bread"));
        QCOMPARE(results[1].second, QStringLiteral("about bread"));
    }
    void tagFilterOpensEveryFolderThatMatches() {
        Fixture f;
        f.write("deep/in/A.md", "#x\n");
        f.write("B.md", "#y\n");
        QVERIFY(f.notebook.filterByTag(QStringLiteral("#X")));
        const auto rows = f.notebook.sidebarRows();
        QCOMPARE(rows.size(), 3);   // deep/, in/, A
        QVERIFY(rows[0].folder && rows[0].expanded);
        QVERIFY(!rows[2].folder);
        QVERIFY(!f.notebook.filterByTag(QStringLiteral("nope")));
        QVERIFY(!f.notebook.activeTag());
    }
    void captureAppendsToTheInbox() {
        Fixture f;
        QVERIFY(f.notebook.appendTo(id("Inbox.md"), "- first"));
        QVERIFY(f.notebook.appendTo(id("Inbox.md"), "- second"));
        QCOMPARE(f.notebook.vault()->read(id("Inbox.md"))->body, QStringLiteral("- first\n- second\n"));
        f.notebook.loadNote(id("Inbox.md"));
        QVERIFY(f.notebook.appendTo(id("Inbox.md"), "- third"));
        QVERIFY(f.notebook.isDirty());
        QCOMPARE(f.notebook.openNote()->body, QStringLiteral("- first\n- second\n- third\n"));
    }
    void configRoundTripsAndDefaultsToTheNas() {
        Fixture f;
        QVERIFY(!f.notebook.syncServer());   // no token → off
        QCOMPARE(*f.notebook.embeddingUrl(), QStringLiteral("http://mattnas:8081"));
        f.notebook.config().syncToken = QStringLiteral("a-token-of-at-least-thirty-two-chars");
        QCOMPARE(f.notebook.syncServer()->first, QStringLiteral("http://mattnas:8082"));
        f.notebook.config().embeddingUrl = QString();
        QVERIFY(!f.notebook.embeddingUrl());
        f.notebook.config().syncUrl = QStringLiteral("http://100.1.2.3:8082/");
        QCOMPARE(f.notebook.syncServer()->first, QStringLiteral("http://100.1.2.3:8082"));
        QVERIFY(f.notebook.saveConfig());
        Notebook again;
        again.loadConfig(f.dir.path() + "/config/brain/config.json");
        QCOMPARE(again.config(), f.notebook.config());
        QCOMPARE(*again.config().vault, f.dir.path() + "/vault");
    }
    void frontmatterEditsFromTheRail() {
        Fixture f;
        f.write("A.md", "---\nother: kept\n---\nbody\n");
        f.notebook.loadNote(id("A.md"));
        f.notebook.setOpenTags({"a", "b"});
        f.notebook.setOpenAliases({"x"});
        QCOMPARE(f.notebook.saveNow(), Notebook::Saved::Written);
        QCOMPARE(f.notebook.vault()->read(id("A.md"))->toText(), QStringLiteral("---\nother: kept\ntags: [a, b]\naliases: [x]\n---\nbody\n"));
        QCOMPARE(f.notebook.index().tagsOf(id("A.md")), (QStringList{"a", "b"}));
    }
};

QTEST_APPLESS_MAIN(TestNotebook)
#include "test_notebook.moc"
