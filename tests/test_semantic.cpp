#include <QtTest>

#include "semantic.h"

using namespace brain;
using namespace brain::semantic;

namespace {
Index build(const QVector<std::pair<QString, QString>> &notes) {
    QVector<Note> out;
    for (const auto &[path, body] : notes) out.append(Note::fromText(NoteId::fromRelative(path), body));
    return Index::build(out);
}

class FakeEmbedder : public Embedder {
public:
    int calls = 0;
    bool fail = false;
    QString model() const override { return "fake"; }
    std::optional<Chunks> embed(const QVector<QString> &texts, EmbedError *error) override {
        ++calls;
        if (fail) { if (error) error->message = "down"; return std::nullopt; }
        Chunks out;
        for (const QString &t : texts) out.append({float(t.size()), 1.0f});
        return out;
    }
};

class FakeShared : public Shared {
public:
    QMap<Digest, Chunks> held;
    int published = 0;
    std::optional<QVector<std::pair<Digest, Chunks>>> fetch(const QVector<Digest> &digests) override {
        QVector<std::pair<Digest, Chunks>> out;
        for (Digest d : digests) if (held.contains(d)) out.append({d, held.value(d)});
        return out;
    }
    bool publish(const QVector<std::pair<Digest, Chunks>> &entries) override { published += entries.size(); for (const auto &e : entries) held.insert(e.first, e.second); return true; }
};
}

class TestSemantic : public QObject {
    Q_OBJECT
private slots:
    void fnv1aMatchesTheRustClient() {
        // FNV-1a test vectors: "" and "a".
        QCOMPARE(fnv1a(""), 0xcbf29ce484222325ULL);
        QCOMPARE(fnv1a("a"), 0xaf63dc4c8601ec8cULL);
    }
    void chunksSplitOnHeadingsAndBudget() {
        QCOMPARE(chunks("T", "one\n\ntwo"), QVector<QString>{"T\n\none\n\ntwo"});
        QCOMPARE(chunks("T", "one\n\n# two\n\nthree").size(), 2);
        QCOMPARE(chunks("T", "").size(), 1);   // the title alone
        const QString huge = QString("x").repeated(CHUNK_CHARS * 2 + 10);
        for (const QString &c : chunks("Long", huge)) QVERIFY(c.size() <= CHUNK_CHARS + 6 + 1);
        QCOMPARE(chunks("Huge", QString("é").repeated(CHUNK_CHARS * 40)).size(), MAX_CHUNKS);
    }
    void digestIgnoresWhatTheModelNeverSees() {
        QCOMPARE(digestOf("T", "a\n\nb"), digestOf("T", "a\n\n\n\nb  "));
        QVERIFY(digestOf("T", "a") != digestOf("U", "a"));
    }
    void planNoticesMovesEditsAndDrops() {
        Store store;
        store.model = "fake";
        store.insert(NoteId::fromRelative("A.md"), digestOf("A", "same"), {{1, 0}});
        store.insert(NoteId::fromRelative("B.md"), digestOf("B", "old"), {{1, 0}});
        store.insert(NoteId::fromRelative("Gone.md"), 99, {{1, 0}});
        const QVector<Wanted> wanted = {{NoteId::fromRelative("Moved/A.md"), digestOf("A", "same")}, {NoteId::fromRelative("B.md"), digestOf("B", "new")}, {NoteId::fromRelative("C.md"), 7}};
        const Plan p = plan(store, wanted);
        QCOMPARE(p.moved.size(), 1);
        QCOMPARE(p.moved[0].to.str(), QStringLiteral("Moved/A.md"));
        QCOMPARE(p.embed.size(), 2);
        QCOMPARE(p.drop, QVector<NoteId>{NoteId::fromRelative("Gone.md")});
        store.apply(p);
        QVERIFY(store.contains(NoteId::fromRelative("Moved/A.md")));
        QVERIFY(!store.contains(NoteId::fromRelative("Gone.md")));
    }
    void nearestUsesTheBestChunkAndAFloor() {
        Store store;
        store.insert(NoteId::fromRelative("A.md"), 1, {{0.0f, 1.0f}, {1.0f, 0.0f}});
        store.insert(NoteId::fromRelative("B.md"), 2, {{0.7f, 0.7f}});
        const auto near = store.nearest({2.0f, 0.0f}, 0.9f, 10);
        QCOMPARE(near.size(), 1);
        QCOMPARE(near[0].first.title(), QStringLiteral("A"));
        QVERIFY(store.nearest({0.0f, -1.0f}, 0.55f, 10).isEmpty());
    }
    void storeRoundTripsWithBigDigests() {
        QTemporaryDir dir;
        Store store;
        store.model = "nomic";
        store.insert(NoteId::fromRelative("A.md"), 0xFFFFFFFFFFFFFFF0ULL, {{0.5f, 0.5f}});
        QVERIFY(store.save(dir.path() + "/nested/vectors.json"));
        const Store read = Store::load(dir.path() + "/nested/vectors.json");
        QCOMPARE(read.model, QStringLiteral("nomic"));
        QCOMPARE(read.get(NoteId::fromRelative("A.md"))->digest, 0xFFFFFFFFFFFFFFF0ULL);
        QCOMPARE(read.get(NoteId::fromRelative("A.md"))->chunks[0].size(), 2);
        QVERIFY(Store::load(dir.path() + "/missing.json").isEmpty());
        // A Rust-written store carries the digest as a bare integer.
        QFile f(dir.path() + "/rust.json");
        f.open(QIODevice::WriteOnly);
        f.write("{\"model\":\"m\",\"notes\":{\"A.md\":{\"digest\":18446744073709551600,\"chunks\":[[0.1,0.2]]}}}");
        f.close();
        QCOMPARE(Store::load(dir.path() + "/rust.json").get(NoteId::fromRelative("A.md"))->digest, 18446744073709551600ULL);
    }
    void catchUpFetchesBeforeEmbeddingAndPublishesAfter() {
        const Index index = build({{"A.md", "alpha"}, {"B.md", "beta"}});
        FakeEmbedder embedder;
        FakeShared shared;
        shared.held.insert(digestOf("A", "alpha"), {{1, 1}});
        Store store;
        const Report r = catchUp(store, index, embedder, &shared);
        QCOMPARE(r.fetched, 1);
        QCOMPARE(r.embedded, 1);
        QCOMPARE(embedder.calls, 1);
        QCOMPARE(shared.published, 1);
        QCOMPARE(store.size(), 2);
        QVERIFY(catchUp(store, index, embedder, &shared).isQuiet());
    }
    void aServerThatStopsLeavesTheRestPending() {
        const Index index = build({{"A.md", "alpha"}, {"B.md", "beta"}});
        FakeEmbedder embedder;
        embedder.fail = true;
        Store store;
        const Report r = catchUp(store, index, embedder);
        QCOMPARE(r.embedded, 0);
        QCOMPARE(r.pending, 2);
        QVERIFY(store.isEmpty());
    }
    void aModelChangeEmptiesTheStore() {
        Store store;
        store.model = "old";
        store.insert(NoteId::fromRelative("A.md"), 1, {{1, 0}});
        QVERIFY(store.setModel("new"));
        QVERIFY(store.isEmpty());
        QVERIFY(!store.setModel("new"));
    }
};

QTEST_APPLESS_MAIN(TestSemantic)
#include "test_semantic.moc"
