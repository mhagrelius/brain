#include <QtTest>

#include "frontmatter.h"
#include "note.h"

using namespace brain;

class TestFrontmatter : public QObject {
    Q_OBJECT
private slots:
    void aNoteRoundTripsByteForByte() {
        for (const QString source : {QStringLiteral("# Title\n\nProse.\n"), QStringLiteral("---\ntags: [rust]\n---\n\n# Title\n"), QString(), QStringLiteral("no trailing newline"), QStringLiteral("---\ntags:\n  - rust\n---\nbody"), QStringLiteral("---\nweird: \"quoted\"\ncreated: last tuesday\n---\n"), QStringLiteral("--- \ntags: [a]\n---")}) {
            const Note note = Note::fromText(NoteId::fromRelative("n.md"), source);
            QCOMPARE(note.toText(), source);
        }
    }
    void theFourKeysAreRead() {
        auto [fm, body] = Frontmatter::split("---\ntags: [a, \"b c\"]\naliases:\n  - x\ncreated: 2026-08-14\nother: kept\n---\nbody\n");
        QVERIFY(fm);
        QCOMPARE(fm->tags, (QStringList{"a", "b c"}));
        QCOMPARE(fm->aliases, QStringList{"x"});
        QCOMPARE(fm->created, QDate(2026, 8, 14));
        QVERIFY(!fm->updated);
        QCOMPARE(body, QStringLiteral("body\n"));
    }
    void anUnterminatedBlockIsNotFrontmatter() {
        auto [fm, body] = Frontmatter::split("---\ntags: [a]\nbody");
        QVERIFY(!fm);
        QCOMPARE(body, QStringLiteral("---\ntags: [a]\nbody"));
    }
    void aChangedValueIsRenderedCanonicallyAndOthersKept() {
        auto [fm, body] = Frontmatter::split("---\ntags:   [a]\nother: kept\n---\n");
        fm->tags = {"a", "b"};
        QCOMPARE(fm->render(), QStringLiteral("---\ntags: [a, b]\nother: kept\n---\n"));
        fm->tags.clear();
        QCOMPARE(fm->render(), QStringLiteral("---\nother: kept\n---\n"));
    }
    void aNewKeyGoesAfterWhatWasThere() {
        auto [fm, body] = Frontmatter::split("---\nother: kept\n---\n");
        fm->aliases = {"z"};
        QCOMPARE(fm->render(), QStringLiteral("---\nother: kept\naliases: [z]\n---\n"));
    }
    void clearingTheLastValueRemovesTheBlock() {
        auto [fm, body] = Frontmatter::split("---\ntags: [a]\n---\nbody");
        fm->tags.clear();
        QCOMPARE(fm->render(), QString());
        Frontmatter fresh;
        QVERIFY(fresh.isEmpty());
        fresh.tags = {"x"};
        QCOMPARE(fresh.render(), QStringLiteral("---\ntags: [x]\n---\n"));
    }
    void tagsComeFromBothPlaces() {
        const Note note = Note::fromText(NoteId::fromRelative("n.md"), "---\ntags: [Rust]\n---\nAbout #learning and #rust again.\n");
        QCOMPARE(note.tags(), (QStringList{"Rust", "learning"}));
    }
    void anExcerptSkipsTheTitleHeading() {
        QCOMPARE(Note::fromText(NoteId::fromRelative("Rust ownership.md"), "# Rust ownership\n\nMoves are **destructive**.\n").excerpt(80), QStringLiteral("Moves are destructive."));
        QCOMPARE(Note::fromText(NoteId::fromRelative("n.md"), "🎉🎉🎉🎉 and more").excerpt(8), QStringLiteral("🎉🎉🎉🎉…"));
    }
    void idsAreRelativePathsWithForwardSlashes() {
        const NoteId id = NoteId::fromRelative("Meetings/standup.md");
        QCOMPARE(id.title(), QStringLiteral("standup"));
        QCOMPARE(id.folder(), QStringLiteral("Meetings"));
        QCOMPARE(NoteId::fromRelative("note.md").folder(), QString());
        QCOMPARE(NoteId::fromRelative("./a//b.md").str(), QStringLiteral("a/b.md"));
    }
};

QTEST_APPLESS_MAIN(TestFrontmatter)
#include "test_frontmatter.moc"
