#include <QtTest>

#include "sync.h"

using namespace brain;
using namespace brain::sync;

namespace {
NoteId id(const QString &p) { return NoteId::fromRelative(p); }
Snapshot snapshot(const QVector<std::pair<QString, QString>> &notes) {
    Snapshot out;
    for (const auto &[path, text] : notes) out.insert(id(path), hashOf(text));
    return out;
}
Plan planOf(const Snapshot &base, const Snapshot &local, const Snapshot &remote) { return plan(base, local, remote, "phone", "2026-08-04"); }

// The server as a map, so every branch of a pass runs against nothing but memory.
class FakeRemote : public Remote {
public:
    QMap<NoteId, QString> notes;
    bool down = false;
    int puts = 0;
    std::optional<Snapshot> list(QString *error) override {
        if (down) { if (error) *error = "down"; return std::nullopt; }
        Snapshot out;
        for (auto it = notes.cbegin(); it != notes.cend(); ++it) out.insert(it.key(), hashOf(it.value()));
        return out;
    }
    std::optional<QVector<std::pair<NoteId, QString>>> get(const QVector<NoteId> &ids, QString *) override {
        QVector<std::pair<NoteId, QString>> out;
        for (const NoteId &i : ids) if (notes.contains(i)) out.append({i, notes.value(i)});
        return out;
    }
    Put put(const NoteId &i, const QString &text, std::optional<Hash> base) override {
        ++puts;
        Put p;
        const std::optional<Hash> current = notes.contains(i) ? std::optional<Hash>(hashOf(notes.value(i))) : std::nullopt;
        if (current != base) { p.kind = Put::Stale; p.hash = current; return p; }
        notes.insert(i, text);
        p.kind = Put::Done;
        p.hash = hashOf(text);
        return p;
    }
    Put remove(const NoteId &i, std::optional<Hash> base) override {
        Put p;
        const std::optional<Hash> current = notes.contains(i) ? std::optional<Hash>(hashOf(notes.value(i))) : std::nullopt;
        if (current != base) { p.kind = Put::Stale; p.hash = current; return p; }
        notes.remove(i);
        p.kind = Put::Done;
        return p;
    }
};
}

class TestSync : public QObject {
    Q_OBJECT
private slots:
    void aQuietVaultPlansNothing() {
        const Snapshot both = snapshot({{"A.md", "one"}, {"B.md", "two"}});
        QVERIFY(planOf(both, both, both).isEmpty());
    }
    void aFirstPassPushesAndPulls() {
        const Plan p = planOf({}, snapshot({{"A.md", "mine"}}), snapshot({{"B.md", "theirs"}}));
        QCOMPARE(p.push, QVector<NoteId>{id("A.md")});
        QCOMPARE(p.pull, QVector<NoteId>{id("B.md")});
        QVERIFY(p.conflicts.isEmpty());
    }
    void anEditOnOneSideMovesOneWay() {
        const Snapshot base = snapshot({{"A.md", "before"}}), edited = snapshot({{"A.md", "after"}});
        QCOMPARE(planOf(base, edited, base).push, QVector<NoteId>{id("A.md")});
        QCOMPARE(planOf(base, base, edited).pull, QVector<NoteId>{id("A.md")});
    }
    void aDeletionNeverWinsOverAnEdit() {
        const Snapshot base = snapshot({{"A.md", "before"}}), edited = snapshot({{"A.md", "after"}});
        const Plan a = planOf(base, {}, edited);
        QCOMPARE(a.pull, QVector<NoteId>{id("A.md")});
        QVERIFY(a.deleteLocal.isEmpty());
        const Plan b = planOf(base, edited, {});
        QCOMPARE(b.push, QVector<NoteId>{id("A.md")});
        QVERIFY(b.deleteRemote.isEmpty());
        QVERIFY(planOf(base, {}, {}).isEmpty());
    }
    void bothSidesEditingIsAConflictBesideTheOriginal() {
        const Snapshot base = snapshot({{"n/A.md", "before"}});
        const Plan p = planOf(base, snapshot({{"n/A.md", "mine"}}), snapshot({{"n/A.md", "theirs"}}));
        QCOMPARE(p.conflicts.size(), 1);
        QCOMPARE(p.conflicts[0].copy.str(), QStringLiteral("n/A (conflict 2026-08-04 from phone).md"));
        QVERIFY(planOf(base, snapshot({{"n/A.md", "same"}}), snapshot({{"n/A.md", "same"}})).isEmpty());
    }
    void aRenameElsewhereIsAppliedAsARename() {
        const Snapshot base = snapshot({{"Ownership.md", "the text"}}), renamed = snapshot({{"Borrowing.md", "the text"}});
        const Plan p = planOf(base, base, renamed);
        QCOMPARE(p.renameLocal.size(), 1);
        QCOMPARE(p.renameLocal[0].second.str(), QStringLiteral("Borrowing.md"));
        QVERIFY(p.pull.isEmpty() && p.deleteLocal.isEmpty());
        const Plan q = planOf(base, renamed, base);
        QCOMPARE(q.push, QVector<NoteId>{id("Borrowing.md")});
        QCOMPARE(q.deleteRemote, QVector<NoteId>{id("Ownership.md")});
    }
    void aWholePassAgainstAMap() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        vault.create(id("A.md"), "mine");
        FakeRemote remote;
        remote.notes.insert(id("B.md"), "theirs");
        QString error;
        auto incoming = gather(vault, {}, remote, "phone", "2026-08-04", &error);
        QVERIFY(incoming);
        auto [agreed, report] = apply(vault, *incoming, nullptr, "phone", "2026-08-04");
        QCOMPARE(report.pushed, 1);
        QCOMPARE(report.pulled, 1);
        QCOMPARE(agreed.size(), 2);
        QCOMPARE(vault.read(id("B.md"))->body, QStringLiteral("theirs"));
        // The steady state is quiet.
        auto again = gather(vault, agreed, remote, "phone", "2026-08-04", &error);
        auto [agreed2, report2] = apply(vault, *again, nullptr, "phone", "2026-08-04");
        QVERIFY(report2.isQuiet());
        QCOMPARE(agreed2, agreed);
    }
    void aPullAimedAtTheOpenDirtyNoteBecomesACopy() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        vault.create(id("A.md"), "v1");
        const Snapshot base = snapshotOf(vault);
        FakeRemote remote;
        remote.notes.insert(id("A.md"), "v2");
        QString error;
        auto incoming = gather(vault, base, remote, "phone", "2026-08-04", &error);
        const NoteId open = id("A.md");
        auto [agreed, report] = apply(vault, *incoming, &open, "phone", "2026-08-04");
        QCOMPARE(report.conflicted, 1);
        QCOMPARE(vault.read(id("A.md"))->body, QStringLiteral("v1"));
        QVERIFY(vault.read(id("A (conflict 2026-08-04 from phone).md")));
        QCOMPARE(agreed.value(id("A.md")), base.value(id("A.md")));   // the base did not advance
    }
    void aServerThatIsDownIsNotAnError() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        FakeRemote remote;
        remote.down = true;
        QString error;
        QVERIFY(!gather(vault, {}, remote, "phone", "2026-08-04", &error));
        QCOMPARE(error, QStringLiteral("down"));
    }
    void aStaleWriteIsRetriedNextPass() {
        QTemporaryDir dir;
        const Vault vault(dir.path());
        vault.create(id("A.md"), "mine");
        FakeRemote remote;
        remote.notes.insert(id("A.md"), "moved on");
        const Snapshot base = snapshot({{"A.md", "before"}});
        QString error;
        auto incoming = gather(vault, base, remote, "phone", "2026-08-04", &error);
        // Both changed → a conflict copy lands, nothing is pushed.
        auto [agreed, report] = apply(vault, *incoming, nullptr, "phone", "2026-08-04");
        QCOMPARE(report.conflicted, 1);
        QCOMPARE(report.pushed, 0);
    }
    void baseSurvivesBigHashesAndRustsFormat() {
        QTemporaryDir dir;
        Snapshot base;
        base.insert(id("A.md"), 0xFFFFFFFFFFFFFFF0ULL);
        QVERIFY(saveBase(base, dir.path() + "/.brain/sync.json"));
        QCOMPARE(loadBase(dir.path() + "/.brain/sync.json"), base);
        QFile f(dir.path() + "/rust.json");
        f.open(QIODevice::WriteOnly);
        f.write("{\"A.md\":18446744073709551600,\"B.md\":7}");
        f.close();
        const Snapshot rust = loadBase(dir.path() + "/rust.json");
        QCOMPARE(rust.value(id("A.md")), 18446744073709551600ULL);
        QCOMPARE(rust.value(id("B.md")), 7ULL);
        QVERIFY(loadBase(dir.path() + "/missing.json").isEmpty());
    }
};

QTEST_APPLESS_MAIN(TestSync)
#include "test_sync.moc"
