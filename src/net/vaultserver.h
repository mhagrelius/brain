#pragma once

// The client for brain-server: the vault routes (sync::Remote) and the
// vector routes (semantic::Shared). Same service, same token. The wire
// format is pinned by tests on both sides; u64 hashes travel as bare JSON
// integers, which is why the bodies are built by hand.

#include "semantic.h"
#include "sync.h"

#include <QString>

namespace brain::net {

class VaultServer : public sync::Remote, public semantic::Shared {
public:
    VaultServer(const QString &url, const QString &token, const QString &model = QString());
    const QString &url() const { return m_url; }

    std::optional<sync::Snapshot> list(QString *error) override;
    std::optional<QVector<std::pair<NoteId, QString>>> get(const QVector<NoteId> &ids, QString *error) override;
    sync::Put put(const NoteId &id, const QString &text, std::optional<sync::Hash> base) override;
    sync::Put remove(const NoteId &id, std::optional<sync::Hash> base) override;

    std::optional<QVector<std::pair<semantic::Digest, semantic::Chunks>>> fetch(const QVector<semantic::Digest> &digests) override;
    bool publish(const QVector<std::pair<semantic::Digest, semantic::Chunks>> &entries) override;

    // Reachable at all? /health needs no token.
    static bool health(const QString &url, int *vectors, QString *error);

    // Exposed for the wire-format tests.
    static QByteArray fetchBody(const QString &model, const QVector<semantic::Digest> &digests);
    static QByteArray publishBody(const QString &model, const QVector<std::pair<semantic::Digest, semantic::Chunks>> &entries);
    static QByteArray putBody(const NoteId &id, const QString &text, std::optional<sync::Hash> base);
    static std::optional<sync::Snapshot> parseList(const QByteArray &body);
    static std::optional<QVector<std::pair<semantic::Digest, semantic::Chunks>>> parseFetch(const QByteArray &body);
    static sync::Put parseWrote(int status, const QByteArray &body);

private:
    std::optional<std::pair<int, QByteArray>> send(const QString &method, const QString &route, const QByteArray &body, QString *error) const;
    sync::Put wrote(const QString &route, const QByteArray &body);
    QString m_url;
    QString m_token;
    QString m_model;
};

} // namespace brain::net
