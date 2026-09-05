#include <QtTest>

#include "index.h"

using namespace brain;

namespace {
Index build(const QVector<std::pair<QString, QString>> &notes) {
    QVector<Note> out;
    for (const auto &[path, body] : notes) out.append(Note::fromText(NoteId::fromRelative(path), body));
    return Index::build(out);
}
}

class TestIndex : public QObject {
    Q_OBJECT
private slots:
    void resolvesByPathTitleThenAlias() {
        const Index index = build({{"Meetings/Standup.md", ""}, {"Standup.md", ""}, {"Rust.md", "---\naliases: [ownership]\n---\n"}});
        QCOMPARE(index.resolve("Meetings/Standup").note.str(), QStringLiteral("Meetings/Standup.md"));
        QCOMPARE(index.resolve("standup").kind, Resolution::Ambiguous);
        const NoteId from = NoteId::fromRelative("Standup.md");
        QCOMPARE(index.resolve("standup", &from).note.str(), QStringLiteral("Standup.md"));
        QCOMPARE(index.resolve("OWNERSHIP").note.str(), QStringLiteral("Rust.md"));
        QCOMPARE(index.resolve("nothing").kind, Resolution::Missing);
    }
    void backlinksCarryTheStrippedLine() {
        const Index index = build({{"A.md", "see [[B|it]] **soon**\nother"}, {"B.md", ""}});
        const auto back = index.backlinks(NoteId::fromRelative("B.md"));
        QCOMPARE(back.size(), 1);
        QCOMPARE(back[0].from.str(), QStringLiteral("A.md"));
        QCOMPARE(back[0].context, QStringLiteral("see it soon"));
        QCOMPARE(index.missing().size(), 0);
    }
    void missingLinksAreReported() {
        const Index index = build({{"A.md", "[[Nowhere]]"}});
        QVERIFY(index.missing().contains("Nowhere"));
    }
    void nestedTagsCountTowardsParents() {
        const Index index = build({{"A.md", "#project/brain"}, {"B.md", "---\ntags: [Project]\n---\n"}});
        const auto tags = index.tags();
        QCOMPARE(tags.size(), 2);
        QCOMPARE(tags[0].first, QStringLiteral("project"));
        QCOMPARE(tags[0].second, 2);
        QCOMPARE(tags[1].first, QStringLiteral("project/brain"));
        QCOMPARE(index.notesTagged("#Project").size(), 2);
        QCOMPARE(index.notesTagged("project/brain").size(), 1);
    }
    void renameKeepsTheEntryAndRewiresBacklinks() {
        Index index = build({{"A.md", "[[B]]"}, {"B.md", "text"}});
        index.rename(NoteId::fromRelative("B.md"), NoteId::fromRelative("C.md"));
        QVERIFY(index.contains(NoteId::fromRelative("C.md")));
        QVERIFY(!index.contains(NoteId::fromRelative("B.md")));
        QCOMPARE(index.resolve("C").note.str(), QStringLiteral("C.md"));
        QCOMPARE(index.text(NoteId::fromRelative("C.md")), QStringLiteral("text"));
    }
    void excerptsAndAttachments() {
        const Index index = build({{"A.md", "# A\n\nFirst **real** line.\n![[pic.png]]"}});
        QCOMPARE(index.excerpt(NoteId::fromRelative("A.md")), QStringLiteral("First real line."));
        QVERIFY(index.referencedAttachments().contains("pic.png"));
    }
};

QTEST_APPLESS_MAIN(TestIndex)
#include "test_index.moc"
