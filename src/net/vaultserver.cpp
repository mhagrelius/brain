#include "vaultserver.h"

#include "http.h"
#include "jsonutil.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

namespace brain::net {

namespace {
QByteArray jsonString(const QString &s) { return QJsonDocument(QJsonArray{s}).toJson(QJsonDocument::Compact).mid(1).chopped(1); }
QByteArray chunksJson(const semantic::Chunks &chunks) {
    QByteArray out = "[";
    for (int i = 0; i < chunks.size(); ++i) {
        if (i) out += ',';
        out += '[';
        for (int j = 0; j < chunks[i].size(); ++j) {
            if (j) out += ',';
            out += QByteArray::number(double(chunks[i][j]), 'g', 9);
        }
        out += ']';
    }
    return out + "]";
}
}

VaultServer::VaultServer(const QString &url, const QString &token, const QString &model) : m_url(url.trimmed()), m_token(token), m_model(model) {
    while (m_url.endsWith(u'/')) m_url.chop(1);
}

std::optional<std::pair<int, QByteArray>> VaultServer::send(const QString &method, const QString &route, const QByteArray &body, QString *error) const {
    Request request;
    request.method = method;
    request.url = m_url + route;
    request.body = body;
    request.bearer = m_token;
    request.timeoutMs = 30000;
    QString message;
    const auto response = net::send(request, &message);
    if (!response) { if (error) *error = QStringLiteral("no answer from the vault server: ") + message; return std::nullopt; }
    return std::make_pair(response->status, response->body);
}

bool VaultServer::health(const QString &url, int *vectors, QString *error) {
    QString clean = url.trimmed();
    while (clean.endsWith(u'/')) clean.chop(1);
    Request request;
    request.url = clean + QStringLiteral("/health");
    request.timeoutMs = 4000;
    const auto response = net::send(request, error);
    if (!response) return false;
    if (response->status != 200) { if (error) *error = QStringLiteral("the vault server said ") + QString::number(response->status); return false; }
    if (vectors) *vectors = QJsonDocument::fromJson(response->body).object().value(QStringLiteral("vectors")).toInt();
    return true;
}

// ---- vault routes -----------------------------------------------------------

std::optional<sync::Snapshot> VaultServer::parseList(const QByteArray &body) {
    const QJsonObject root = QJsonDocument::fromJson(json::quoteBigIntegers(body)).object();
    if (!root.contains(QStringLiteral("notes"))) return std::nullopt;
    sync::Snapshot out;
    for (const QJsonValue &v : root.value(QStringLiteral("notes")).toArray()) {
        const QJsonObject o = v.toObject();
        out.insert(NoteId::fromRelative(o.value(QStringLiteral("id")).toString()), json::toU64(o.value(QStringLiteral("hash"))));
    }
    return out;
}

std::optional<sync::Snapshot> VaultServer::list(QString *error) {
    const auto reply = send(QStringLiteral("GET"), QStringLiteral("/notes"), QByteArray(), error);
    if (!reply) return std::nullopt;
    if (reply->first < 200 || reply->first >= 300) { if (error) *error = QStringLiteral("the vault server said ") + QString::number(reply->first); return std::nullopt; }
    const auto listed = parseList(reply->second);
    if (!listed && error) *error = QStringLiteral("the vault server sent something that is not a listing");
    return listed;
}

std::optional<QVector<std::pair<NoteId, QString>>> VaultServer::get(const QVector<NoteId> &ids, QString *error) {
    QVector<std::pair<NoteId, QString>> out;
    if (ids.isEmpty()) return out;
    QJsonArray asked;
    for (const NoteId &id : ids) asked.append(id.str());
    const QByteArray body = QJsonDocument(QJsonObject{{QStringLiteral("ids"), asked}}).toJson(QJsonDocument::Compact);
    const auto reply = send(QStringLiteral("POST"), QStringLiteral("/notes/get"), body, error);
    if (!reply) return std::nullopt;
    if (reply->first < 200 || reply->first >= 300) { if (error) *error = QStringLiteral("the vault server said ") + QString::number(reply->first); return std::nullopt; }
    const QJsonObject root = QJsonDocument::fromJson(json::quoteBigIntegers(reply->second)).object();
    for (const QJsonValue &v : root.value(QStringLiteral("notes")).toArray()) {
        const QJsonObject o = v.toObject();
        out.append({NoteId::fromRelative(o.value(QStringLiteral("id")).toString()), o.value(QStringLiteral("text")).toString()});
    }
    return out;
}

QByteArray VaultServer::putBody(const NoteId &id, const QString &text, std::optional<sync::Hash> base) {
    QByteArray body = "{\"id\":" + jsonString(id.str()) + ",\"text\":" + jsonString(text) + ",\"base\":";
    body += base ? QByteArray::number(qulonglong(*base)) : QByteArray("null");
    return body + "}";
}

sync::Put VaultServer::parseWrote(int status, const QByteArray &body) {
    sync::Put put;
    const QJsonObject root = QJsonDocument::fromJson(json::quoteBigIntegers(body)).object();
    if (status >= 200 && status < 300) {
        put.kind = sync::Put::Done;
        put.hash = json::toU64(root.value(QStringLiteral("hash")));
        return put;
    }
    if (status == 409) {
        put.kind = sync::Put::Stale;
        const QJsonValue current = root.value(QStringLiteral("current"));
        if (!current.isNull() && !current.isUndefined()) put.hash = json::toU64(current);
        return put;
    }
    put.kind = sync::Put::Failed;
    put.error = QStringLiteral("the vault server said ") + QString::number(status);
    return put;
}

sync::Put VaultServer::wrote(const QString &route, const QByteArray &body) {
    QString error;
    const auto reply = send(QStringLiteral("POST"), route, body, &error);
    if (!reply) { sync::Put put; put.kind = sync::Put::Failed; put.error = error; return put; }
    return parseWrote(reply->first, reply->second);
}

sync::Put VaultServer::put(const NoteId &id, const QString &text, std::optional<sync::Hash> base) {
    return wrote(QStringLiteral("/notes/put"), putBody(id, text, base));
}

sync::Put VaultServer::remove(const NoteId &id, std::optional<sync::Hash> base) {
    QByteArray body = "{\"id\":" + jsonString(id.str()) + ",\"base\":" + (base ? QByteArray::number(qulonglong(*base)) : QByteArray("null")) + "}";
    return wrote(QStringLiteral("/notes/delete"), body);
}

// ---- vector routes ----------------------------------------------------------

QByteArray VaultServer::fetchBody(const QString &model, const QVector<semantic::Digest> &digests) {
    QByteArray body = "{\"model\":" + jsonString(model) + ",\"digests\":[";
    for (int i = 0; i < digests.size(); ++i) {
        if (i) body += ',';
        body += QByteArray::number(qulonglong(digests[i]));
    }
    return body + "]}";
}

QByteArray VaultServer::publishBody(const QString &model, const QVector<std::pair<semantic::Digest, semantic::Chunks>> &entries) {
    QByteArray body = "{\"model\":" + jsonString(model) + ",\"entries\":[";
    for (int i = 0; i < entries.size(); ++i) {
        if (i) body += ',';
        body += '[' + QByteArray::number(qulonglong(entries[i].first)) + ',' + chunksJson(entries[i].second) + ']';
    }
    return body + "]}";
}

std::optional<QVector<std::pair<semantic::Digest, semantic::Chunks>>> VaultServer::parseFetch(const QByteArray &body) {
    const QJsonObject root = QJsonDocument::fromJson(json::quoteBigIntegers(body)).object();
    if (!root.contains(QStringLiteral("found"))) return std::nullopt;
    QVector<std::pair<semantic::Digest, semantic::Chunks>> out;
    for (const QJsonValue &v : root.value(QStringLiteral("found")).toArray()) {
        const QJsonArray pair = v.toArray();
        if (pair.size() != 2) continue;
        semantic::Chunks chunks;
        for (const QJsonValue &row : pair[1].toArray()) {
            QVector<float> chunk;
            for (const QJsonValue &x : row.toArray()) chunk.append(float(x.toDouble()));
            chunks.append(chunk);
        }
        out.append({json::toU64(pair[0]), chunks});
    }
    return out;
}

std::optional<QVector<std::pair<semantic::Digest, semantic::Chunks>>> VaultServer::fetch(const QVector<semantic::Digest> &digests) {
    if (digests.isEmpty()) return QVector<std::pair<semantic::Digest, semantic::Chunks>>();
    QString error;
    const auto reply = send(QStringLiteral("POST"), QStringLiteral("/fetch"), fetchBody(m_model, digests), &error);
    if (!reply || reply->first < 200 || reply->first >= 300) return std::nullopt;
    return parseFetch(reply->second);
}

bool VaultServer::publish(const QVector<std::pair<semantic::Digest, semantic::Chunks>> &entries) {
    if (entries.isEmpty()) return true;
    QString error;
    const auto reply = send(QStringLiteral("POST"), QStringLiteral("/publish"), publishBody(m_model, entries), &error);
    return reply && reply->first >= 200 && reply->first < 300;
}

} // namespace brain::net
