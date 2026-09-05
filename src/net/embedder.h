#pragma once

// A llama.cpp server that turns text into vectors, over the OpenAI-shaped
// /v1/embeddings route. Connect asks the server what it is serving, because
// the model's name is what the store keys on.

#include "semantic.h"

#include <QString>

#include <optional>

namespace brain::net {

struct Prefixes {
    QString scheme;
    QString document;
    QString query;
};
Prefixes prefixesFor(const QString &model);

class Llama : public semantic::Embedder {
public:
    static std::optional<Llama> connect(const QString &base, QString *error);
    QString model() const override { return m_model + u'+' + m_prefixes.scheme; }
    QString modelName() const { return m_model; }
    std::optional<semantic::Chunks> embed(const QVector<QString> &texts, semantic::EmbedError *error) override;
    std::optional<QVector<float>> embedQuery(const QString &query, semantic::EmbedError *error) override;

    // Exposed for the wire-format tests.
    static std::optional<semantic::Chunks> parseEmbeddings(const QByteArray &body, int expected, QString *error);
    static std::optional<QString> parseModels(const QByteArray &body);

private:
    std::optional<semantic::Chunks> embedPrefixed(const QString &prefix, const QVector<QString> &texts, semantic::EmbedError *error);
    QString m_base;
    QString m_model;
    Prefixes m_prefixes;
};

} // namespace brain::net
