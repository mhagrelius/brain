#pragma once

// Finding notes: fuzzy titles, literal text, BM25 over the words, and the
// reciprocal-rank fusion of BM25 with vectors.

#include "index.h"

#include <QHash>
#include <QString>
#include <QStringList>
#include <QVector>

#include <optional>

namespace brain {

namespace semantic { class Store; }

namespace search {

struct TitleMatch {
    NoteId id;
    int score = 0;
    QVector<int> positions;
};

struct Snippet {
    QString text;
    int start = 0;
    int end = 0;
};

struct TextMatch {
    NoteId id;
    int hits = 0;
    QVector<Snippet> snippets;
};

// Subsequence match: "rsown" finds "Rust ownership".
QVector<TitleMatch> byTitle(const Index &index, const QString &query, int limit);
// Case-insensitive substring match over the stripped text.
QVector<TextMatch> byText(const Index &index, const QString &query, int limit);

QStringList tokenize(const QString &text);

// Okapi BM25 over counts already in memory. Rebuilt whenever the vault changes.
class Bm25 {
public:
    Bm25() = default;
    static Bm25 build(const Index &index);
    QVector<std::pair<NoteId, float>> search(const QString &query, int limit) const;
    int size() const { return m_documents.size(); }

private:
    struct Document {
        NoteId id;
        QHash<QString, quint32> counts;
        float length = 0;
    };
    QVector<Document> m_documents;
    QHash<QString, int> m_frequencies;
    float m_averageLength = 0;
};

constexpr float RRF_K = 60.0f;
constexpr float SEMANTIC_FLOOR = 0.55f;

struct Hit {
    NoteId id;
    float score = 0;
    std::optional<int> lexical;
    std::optional<int> semantic;
    std::optional<float> similarity;
    QString snippet;
};

// Lexical, semantic or both, fused. `queryVector` empty means no model: BM25 alone.
QVector<Hit> hybrid(const Index &index, const Bm25 &lexical, const semantic::Store *store, const QVector<float> &queryVector, const QString &query, int limit);
std::optional<std::pair<int, int>> highlightOf(const QString &snippet, const QString &query);

} // namespace search
} // namespace brain
