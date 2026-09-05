#include <QtTest>

#include "search.h"
#include "semantic.h"

using namespace brain;

namespace {
Index build(const QVector<std::pair<QString, QString>> &notes) {
    QVector<Note> out;
    for (const auto &[path, body] : notes) out.append(Note::fromText(NoteId::fromRelative(path), body));
    return Index::build(out);
}
QStringList titles(const QVector<search::TitleMatch> &m) { QStringList out; for (const auto &x : m) out.append(x.id.title()); return out; }
}

class TestSearch : public QObject {
    Q_OBJECT
private slots:
    void anExactTitleRanksFirst() {
        const Index index = build({{"Rust ownership notes.md", ""}, {"Rust.md", ""}, {"Trusted sources.md", ""}});
        QCOMPARE(titles(search::byTitle(index, "rust", 10)).first(), QStringLiteral("Rust"));
        QCOMPARE(titles(search::byTitle(index, "rsown", 10)).first(), QStringLiteral("Rust ownership notes"));
        QCOMPARE(search::byTitle(index, "zzz", 10).size(), 0);
        QCOMPARE(search::byTitle(index, "", 10).size(), 3);
    }
    void aPathMatchRanksBelowTitles() {
        const Index index = build({{"meet/standup.md", ""}, {"stand.md", ""}});
        QCOMPARE(titles(search::byTitle(index, "meet/stand", 10)), QStringList{"standup"});
    }
    void textSearchIsSubstringWithSnippets() {
        const Index index = build({{"A.md", "line one\n  the Ownership word\nownership again"}, {"Ownership.md", "the owner of this"}});
        const auto m = search::byText(index, "owner", 10);
        QCOMPARE(m.size(), 2);
        QCOMPARE(m[0].id.title(), QStringLiteral("Ownership"));   // titled first
        QCOMPARE(m[1].hits, 2);
        QCOMPARE(m[1].snippets[0].text, QStringLiteral("the Ownership word"));
        QCOMPARE(m[1].snippets[0].start, 4);
        QCOMPARE(m[1].snippets[0].end, 9);
    }
    void tokenizeLowercasesAndSplits() {
        QCOMPARE(search::tokenize("Hello, café-Bar 42!"), (QStringList{"hello", "café", "bar", "42"}));
    }
    void bm25IgnoresStopwordsAndNoise() {
        const Index index = build({{"Sourdough.md", "why is my bread so flat and dense, hydration"}, {"GPU.md", "so the model runs on the gpu"}, {"C.md", "unrelated"}, {"D.md", "more unrelated"}});
        const search::Bm25 bm = search::Bm25::build(index);
        const auto hits = bm.search("why is my bread so flat and dense", 10);
        QVERIFY(!hits.isEmpty());
        QCOMPARE(hits[0].first.title(), QStringLiteral("Sourdough"));
        QCOMPARE(hits.size(), 1);   // the GPU note matched "so" only: noise
        QVERIFY(bm.search("the", 10).isEmpty());
    }
    void titleWordsWeighMore() {
        const Index index = build({{"Rust ownership.md", "a note"}, {"Other.md", "ownership mentioned in passing"}});
        const auto hits = search::Bm25::build(index).search("ownership", 10);
        QCOMPARE(hits[0].first.title(), QStringLiteral("Rust ownership"));
    }
    void hybridFusesRanksAndDegradesToWords() {
        const Index index = build({{"A.md", "bread and flour"}, {"B.md", "engines"}, {"C.md", "bread"}});
        const search::Bm25 bm = search::Bm25::build(index);
        semantic::Store store;
        store.model = "m";
        store.insert(NoteId::fromRelative("B.md"), 1, {{1.0f, 0.0f}});
        store.insert(NoteId::fromRelative("C.md"), 2, {{0.0f, 1.0f}});
        const auto lexical = search::hybrid(index, bm, &store, {}, "bread", 10);
        QCOMPARE(lexical.size(), 2);
        QVERIFY(lexical[0].lexical.has_value());
        QVERIFY(!lexical[0].semantic.has_value());
        const auto fused = search::hybrid(index, bm, &store, {0.0f, 1.0f}, "bread", 10);
        QCOMPARE(fused[0].id.title(), QStringLiteral("C"));   // named by both halves
        QVERIFY(fused[0].semantic.has_value() && fused[0].lexical.has_value());
        QCOMPARE(fused[0].snippet, QStringLiteral("bread"));
        const auto meaning = search::hybrid(index, bm, &store, {1.0f, 0.0f}, "nothing here", 10);
        QCOMPARE(meaning.size(), 1);
        QCOMPARE(meaning[0].id.title(), QStringLiteral("B"));
        QCOMPARE(meaning[0].snippet, QStringLiteral("engines"));   // the excerpt, for a purely semantic hit
    }
    void highlightPrefersTheWholeQuery() {
        QCOMPARE(*search::highlightOf("the Borrow Checker is here", "borrow checker"), std::make_pair(4, 18));
        QCOMPARE(*search::highlightOf("only the checker", "borrow checker"), std::make_pair(9, 16));
        QVERIFY(!search::highlightOf("nothing", "borrow"));
    }
};

QTEST_APPLESS_MAIN(TestSearch)
#include "test_search.moc"
