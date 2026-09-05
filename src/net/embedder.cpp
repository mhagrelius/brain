#include "embedder.h"

#include "http.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

namespace brain::net {

Prefixes prefixesFor(const QString &model) {
    const QString name = model.toLower();
    if (name.contains(QStringLiteral("nomic-embed"))) return {QStringLiteral("nomic"), QStringLiteral("search_document: "), QStringLiteral("search_query: ")};
    if (name.contains(QStringLiteral("e5"))) return {QStringLiteral("e5"), QStringLiteral("passage: "), QStringLiteral("query: ")};
    if (name.contains(QStringLiteral("bge")) && !name.contains(QStringLiteral("m3"))) return {QStringLiteral("bge"), QString(), QStringLiteral("Represent this sentence for searching relevant passages: ")};
    return {QStringLiteral("plain"), QString(), QString()};
}

std::optional<QString> Llama::parseModels(const QByteArray &body) {
    const QJsonObject root = QJsonDocument::fromJson(body).object();
    const QJsonArray data = root.value(QStringLiteral("data")).toArray();
    if (!data.isEmpty()) {
        const QString id = data.first().toObject().value(QStringLiteral("id")).toString();
        if (!id.isEmpty()) return id;
    }
    const QJsonArray models = root.value(QStringLiteral("models")).toArray();
    if (!models.isEmpty()) {
        const QString name = models.first().toObject().value(QStringLiteral("name")).toString();
        if (!name.isEmpty()) return name;
    }
    return std::nullopt;
}

std::optional<Llama> Llama::connect(const QString &rawBase, QString *error) {
    QString base = rawBase.trimmed();
    while (base.endsWith(u'/')) base.chop(1);
    Request request;
    request.url = base + QStringLiteral("/v1/models");
    request.timeoutMs = 5000;
    const auto response = send(request, error);
    if (!response) return std::nullopt;
    if (response->status != 200) { if (error) *error = QStringLiteral("the embedding server said ") + QString::number(response->status); return std::nullopt; }
    const auto model = parseModels(response->body);
    if (!model) { if (error) *error = QStringLiteral("the embedding server named no model"); return std::nullopt; }
    Llama llama;
    llama.m_base = base;
    llama.m_model = *model;
    // The path of the model file is not the name: keep the last segment.
    const QString tail = llama.m_model.section(u'/', -1);
    if (!tail.isEmpty()) llama.m_model = tail;
    llama.m_prefixes = prefixesFor(llama.m_model);
    return llama;
}

std::optional<semantic::Chunks> Llama::parseEmbeddings(const QByteArray &body, int expected, QString *error) {
    const QJsonObject root = QJsonDocument::fromJson(body).object();
    if (root.contains(QStringLiteral("error"))) {
        const QJsonValue e = root.value(QStringLiteral("error"));
        const QString message = e.isObject() ? e.toObject().value(QStringLiteral("message")).toString() : e.toString();
        if (error) *error = message.isEmpty() ? QStringLiteral("the server refused") : message;
        return std::nullopt;
    }
    const QJsonArray data = root.value(QStringLiteral("data")).toArray();
    if (data.isEmpty() && expected > 0) { if (error) *error = QStringLiteral("no embeddings came back"); return std::nullopt; }
    QVector<std::pair<int, QVector<float>>> rows;
    for (const QJsonValue &entry : data) {
        const QJsonObject o = entry.toObject();
        QJsonValue embedding = o.value(QStringLiteral("embedding"));
        // llama.cpp answers a pooled request with [..] and an unpooled one
        // with [[..]]. Take the first row either way.
        if (embedding.isArray() && !embedding.toArray().isEmpty() && embedding.toArray().first().isArray()) embedding = embedding.toArray().first();
        if (!embedding.isArray()) { if (error) *error = QStringLiteral("an entry carried no embedding"); return std::nullopt; }
        QVector<float> vector;
        for (const QJsonValue &v : embedding.toArray()) {
            if (!v.isDouble()) { if (error) *error = QStringLiteral("an embedding held something that is not a number"); return std::nullopt; }
            vector.append(float(v.toDouble()));
        }
        rows.append({o.value(QStringLiteral("index")).toInt(rows.size()), vector});
    }
    if (rows.size() != expected) { if (error) *error = QStringLiteral("asked for %1 embeddings and got %2").arg(expected).arg(rows.size()); return std::nullopt; }
    std::sort(rows.begin(), rows.end(), [](const auto &a, const auto &b) { return a.first < b.first; });
    semantic::Chunks out;
    for (const auto &row : rows) out.append(row.second);
    return out;
}

std::optional<semantic::Chunks> Llama::embedPrefixed(const QString &prefix, const QVector<QString> &texts, semantic::EmbedError *error) {
    QJsonArray input;
    for (const QString &text : texts) input.append(prefix + text);
    Request request;
    request.method = QStringLiteral("POST");
    request.url = m_base + QStringLiteral("/v1/embeddings");
    request.body = QJsonDocument(QJsonObject{{QStringLiteral("input"), input}, {QStringLiteral("model"), m_model}}).toJson(QJsonDocument::Compact);
    request.timeoutMs = 120000;
    QString message;
    const auto response = send(request, &message);
    if (!response) { if (error) error->message = message; return std::nullopt; }
    const auto vectors = parseEmbeddings(response->body, texts.size(), &message);
    if (!vectors) { if (error) error->message = message; return std::nullopt; }
    return vectors;
}

std::optional<semantic::Chunks> Llama::embed(const QVector<QString> &texts, semantic::EmbedError *error) {
    return embedPrefixed(m_prefixes.document, texts, error);
}

std::optional<QVector<float>> Llama::embedQuery(const QString &query, semantic::EmbedError *error) {
    const auto vectors = embedPrefixed(m_prefixes.query, {query}, error);
    if (!vectors || vectors->isEmpty()) return std::nullopt;
    return vectors->first();
}

} // namespace brain::net
