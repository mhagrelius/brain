#include <QtTest>

#include "embedder.h"
#include "jsonutil.h"
#include "vaultserver.h"

using namespace brain;
using namespace brain::net;

// The wire format is pinned on both sides: brain-server defines the same JSON
// in Rust and never sees these types, so each side asserts the exact bytes.
class TestWire : public QObject {
    Q_OBJECT
private slots:
    void putCarriesTheBaseAsABareInteger() {
        QCOMPARE(VaultServer::putBody(NoteId::fromRelative("a/b.md"), "text\n", 18446744073709551600ULL), QByteArray("{\"id\":\"a/b.md\",\"text\":\"text\\n\",\"base\":18446744073709551600}"));
        QCOMPARE(VaultServer::putBody(NoteId::fromRelative("n.md"), "x", std::nullopt), QByteArray("{\"id\":\"n.md\",\"text\":\"x\",\"base\":null}"));
    }
    void fetchAndPublishMatchTheServersShape() {
        QCOMPARE(VaultServer::fetchBody("nomic", {7, 18446744073709551600ULL}), QByteArray("{\"model\":\"nomic\",\"digests\":[7,18446744073709551600]}"));
        QCOMPARE(VaultServer::publishBody("nomic", {{7, {{0.5f, 0.5f}}}}), QByteArray("{\"model\":\"nomic\",\"entries\":[[7,[[0.5,0.5]]]]}"));
    }
    void listAndFetchRepliesParseWithFullPrecision() {
        const auto listed = VaultServer::parseList("{\"notes\":[{\"id\":\"A.md\",\"hash\":18446744073709551600},{\"id\":\"B.md\",\"hash\":7}]}");
        QVERIFY(listed);
        QCOMPARE(listed->value(NoteId::fromRelative("A.md")), 18446744073709551600ULL);
        QCOMPARE(listed->value(NoteId::fromRelative("B.md")), 7ULL);
        QVERIFY(!VaultServer::parseList("{\"error\":\"unauthorised\"}"));
        const auto found = VaultServer::parseFetch("{\"found\":[[18446744073709551600,[[0.5,0.5]]]]}");
        QCOMPARE(found->size(), 1);
        QCOMPARE(found->first().first, 18446744073709551600ULL);
        QCOMPARE(found->first().second[0][1], 0.5f);
    }
    void aWriteAnswersThreeWays() {
        const sync::Put done = VaultServer::parseWrote(200, "{\"hash\":18446744073709551600}");
        QCOMPARE(done.kind, sync::Put::Done);
        QCOMPARE(*done.hash, 18446744073709551600ULL);
        const sync::Put stale = VaultServer::parseWrote(409, "{\"current\":null}");
        QCOMPARE(stale.kind, sync::Put::Stale);
        QVERIFY(!stale.hash);
        const sync::Put staleWith = VaultServer::parseWrote(409, "{\"current\":9}");
        QCOMPARE(*staleWith.hash, 9ULL);
        QCOMPARE(VaultServer::parseWrote(500, "{\"error\":\"could not write\"}").kind, sync::Put::Failed);
    }
    void bigIntegersAreQuotedOnlyWhenBare() {
        QCOMPARE(json::quoteBigIntegers("{\"a\":18446744073709551600,\"b\":[0.019289305433630943,-1.2345678901234567e-05],\"c\":\"18446744073709551600\"}"),
                 QByteArray("{\"a\":\"18446744073709551600\",\"b\":[0.019289305433630943,-1.2345678901234567e-05],\"c\":\"18446744073709551600\"}"));
        QCOMPARE(json::quoteBigIntegers("[7,1234567890123456]"), QByteArray("[7,\"1234567890123456\"]"));
    }
    void embeddingsParseEitherShapeInOrder() {
        QString error;
        const auto pooled = Llama::parseEmbeddings("{\"data\":[{\"embedding\":[9.0],\"index\":1},{\"embedding\":[1.0],\"index\":0}]}", 2, &error);
        QCOMPARE(pooled->size(), 2);
        QCOMPARE((*pooled)[0][0], 1.0f);
        QCOMPARE((*pooled)[1][0], 9.0f);
        const auto unpooled = Llama::parseEmbeddings("{\"data\":[{\"embedding\":[[0.1,0.2]],\"index\":0}]}", 1, &error);
        QCOMPARE((*unpooled)[0].size(), 2);
        QVERIFY(!Llama::parseEmbeddings("{\"data\":[{\"embedding\":[0.1],\"index\":0}]}", 3, &error));
        QVERIFY(!Llama::parseEmbeddings("{\"error\":{\"message\":\"This server does not support embeddings. Start it with `--embeddings`\"}}", 1, &error));
        QVERIFY(error.contains("--embeddings"));
        QVERIFY(!Llama::parseEmbeddings("nonsense", 1, &error));
    }
    void theModelNameComesFromTheServer() {
        QCOMPARE(*Llama::parseModels("{\"models\":[{\"name\":\"nomic-embed-text-v1.5\"}],\"data\":[{\"id\":\"nomic-embed-text-v1.5\"}]}"), QStringLiteral("nomic-embed-text-v1.5"));
        QVERIFY(!Llama::parseModels("{}"));
    }
    void prefixesFollowTheModelCard() {
        QCOMPARE(prefixesFor("nomic-embed-text-v1.5").query, QStringLiteral("search_query: "));
        QCOMPARE(prefixesFor("multilingual-e5-small").document, QStringLiteral("passage: "));
        QCOMPARE(prefixesFor("bge-m3").scheme, QStringLiteral("plain"));
        QVERIFY(prefixesFor("bge-small").document.isEmpty());
    }
};

QTEST_APPLESS_MAIN(TestWire)
#include "test_wire.moc"
