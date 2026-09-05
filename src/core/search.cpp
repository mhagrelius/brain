#include "search.h"

#include "semantic.h"

#include <QMap>

#include <algorithm>
#include <cmath>

namespace brain::search {

namespace {

// Subsequence score, or nullopt if the query does not appear in order.
std::optional<std::pair<int, QVector<int>>> score(const QString &candidate, const QString &query) {
    const QString needles = query.toLower();
    QVector<int> positions;
    int total = 0;
    int at = 0;
    for (int index = 0; index < needles.size(); ++index) {
        const QChar needle = needles[index];
        int found = -1;
        for (int i = at; i < candidate.size(); ++i) {
            if (candidate[i].toLower() == needle) { found = i; break; }
        }
        if (found < 0) return std::nullopt;
        if (found == 0) total += 15;
        else if (!candidate[found - 1].isLetterOrNumber()) total += 10;
        if (index > 0 && !positions.isEmpty() && positions.last() == found - 1) total += 8;
        total -= found - at;
        positions.append(found);
        at = found + 1;
    }
    total -= (candidate.size() - needles.size()) / 4;
    return std::make_pair(total, positions);
}

int leadingSpaces(const QString &line) {
    int n = 0;
    while (n < line.size() && line[n].isSpace()) ++n;
    return n;
}

const QStringList &stopwords() {
    static const QStringList words = {"a", "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "his", "how", "i", "if", "in", "is", "it", "its", "me", "my", "no", "not", "of", "on", "one", "or", "our", "out", "she", "so", "some", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to", "up", "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would", "you", "your"};
    return words;
}

bool isStopword(const QString &term) { return std::binary_search(stopwords().begin(), stopwords().end(), term); }

constexpr float K1 = 1.2f;
constexpr float B = 0.75f;
constexpr quint32 TITLE_WEIGHT = 4;
constexpr float ENOUGH_TO_JUDGE = 4.0f;
constexpr float NOISE = 0.1f;
constexpr int DEPTH = 8;
constexpr int SNIPPETS_PER_NOTE = 3;

bool informative(float containing, float total) { return total < ENOUGH_TO_JUDGE || containing / total < 0.5f; }

QString snippetFor(const Index &index, const NoteId &id, const QString &query) {
    const QStringList terms = tokenize(query);
    const QString text = index.text(id);
    int bestHits = 0;
    QString bestLine;
    for (const QString &raw : text.split(u'\n')) {
        const QString line = raw.trimmed();
        if (line.isEmpty()) continue;
        const QStringList words = tokenize(line);
        int hits = 0;
        for (const QString &term : terms) if (words.contains(term)) ++hits;
        if (hits > bestHits) { bestHits = hits; bestLine = line; }
    }
    if (bestHits > 0) return bestLine.left(200);
    return index.excerpt(id);
}

} // namespace

QStringList tokenize(const QString &text) {
    QStringList terms;
    QString current;
    for (QChar c : text) {
        if (c.isLetterOrNumber()) current.append(c.toLower());
        else if (!current.isEmpty()) { terms.append(current); current.clear(); }
    }
    if (!current.isEmpty()) terms.append(current);
    return terms;
}

QVector<TitleMatch> byTitle(const Index &index, const QString &rawQuery, int limit) {
    const QString query = rawQuery.trimmed();
    QVector<TitleMatch> matches;
    for (const NoteId &id : index.ids()) {
        if (query.isEmpty()) { matches.append(TitleMatch{id, 0, {}}); continue; }
        if (const auto s = score(id.title(), query)) { matches.append(TitleMatch{id, s->first, s->second}); continue; }
        if (const auto s = score(id.str(), query)) matches.append(TitleMatch{id, s->first - 20, {}});
    }
    std::sort(matches.begin(), matches.end(), [](const TitleMatch &a, const TitleMatch &b) { return a.score != b.score ? a.score > b.score : a.id < b.id; });
    if (matches.size() > limit) matches.resize(limit);
    return matches;
}

QVector<TextMatch> byText(const Index &index, const QString &rawQuery, int limit) {
    const QString query = rawQuery.trimmed().toLower();
    if (query.isEmpty()) return {};
    QVector<TextMatch> matches;
    for (const NoteId &id : index.ids()) {
        const QString text = index.text(id);
        int hits = 0;
        QVector<Snippet> snippets;
        for (const QString &line : text.split(u'\n')) {
            const QString lowered = line.toLower();
            int from = 0;
            for (;;) {
                const int at = lowered.indexOf(query, from);
                if (at < 0) break;
                ++hits;
                if (snippets.size() < SNIPPETS_PER_NOTE) {
                    const int start = std::max(0, at - leadingSpaces(line));
                    snippets.append(Snippet{line.trimmed(), start, start + query.size()});
                }
                from = at + query.size();
            }
        }
        if (hits > 0) matches.append(TextMatch{id, hits, snippets});
    }
    auto titled = [&](const NoteId &id) { return id.title().toLower().contains(query); };
    std::sort(matches.begin(), matches.end(), [&](const TextMatch &a, const TextMatch &b) {
        const bool ta = titled(a.id), tb = titled(b.id);
        if (ta != tb) return ta;
        if (a.hits != b.hits) return a.hits > b.hits;
        return a.id < b.id;
    });
    if (matches.size() > limit) matches.resize(limit);
    return matches;
}

Bm25 Bm25::build(const Index &index) {
    Bm25 out;
    for (const NoteId &id : index.ids()) {
        Document doc;
        doc.id = id;
        for (const QString &term : tokenize(index.text(id))) doc.counts[term] += 1;
        for (const QString &term : tokenize(id.str())) doc.counts[term] += TITLE_WEIGHT;
        quint64 length = 0;
        for (auto it = doc.counts.cbegin(); it != doc.counts.cend(); ++it) { length += it.value(); out.m_frequencies[it.key()] += 1; }
        doc.length = float(length);
        out.m_documents.append(doc);
    }
    if (!out.m_documents.isEmpty()) {
        float sum = 0;
        for (const Document &d : out.m_documents) sum += d.length;
        out.m_averageLength = sum / float(out.m_documents.size());
    }
    return out;
}

QVector<std::pair<NoteId, float>> Bm25::search(const QString &query, int limit) const {
    QStringList terms;
    for (const QString &term : tokenize(query)) if (!isStopword(term)) terms.append(term);
    if (terms.isEmpty() || m_documents.isEmpty()) return {};
    const float total = float(m_documents.size());
    QVector<std::pair<NoteId, float>> scored;
    for (const Document &doc : m_documents) {
        float s = 0;
        for (const QString &term : terms) {
            const auto it = doc.counts.find(term);
            if (it == doc.counts.end()) continue;
            const float containing = float(m_frequencies.value(term, 0));
            if (!informative(containing, total)) continue;
            const float idf = std::log((total - containing + 0.5f) / (containing + 0.5f) + 1.0f);
            const float frequency = float(it.value());
            const float normalised = 1.0f - B + B * (doc.length / std::max(m_averageLength, 1.0f));
            s += idf * (frequency * (K1 + 1.0f)) / (frequency + K1 * normalised);
        }
        if (s > 0) scored.append({doc.id, s});
    }
    std::sort(scored.begin(), scored.end(), [](const auto &a, const auto &b) { return a.second != b.second ? a.second > b.second : a.first < b.first; });
    if (!scored.isEmpty()) {
        const float cutoff = scored.first().second * NOISE;
        scored.erase(std::remove_if(scored.begin(), scored.end(), [cutoff](const auto &e) { return e.second < cutoff; }), scored.end());
    }
    if (scored.size() > limit) scored.resize(limit);
    return scored;
}

QVector<Hit> hybrid(const Index &index, const Bm25 &lexical, const semantic::Store *store, const QVector<float> &queryVector, const QString &query, int limit) {
    const int depth = std::max(limit * DEPTH, limit);
    const auto keyword = lexical.search(query, depth);
    QVector<std::pair<NoteId, float>> vectors;
    if (store && !queryVector.isEmpty()) vectors = store->nearest(queryVector, SEMANTIC_FLOOR, depth);

    QMap<NoteId, Hit> fused;
    auto rankIn = [&](const QVector<std::pair<NoteId, float>> &ranking, bool lexicalSide) {
        for (int rank = 0; rank < ranking.size(); ++rank) {
            Hit &hit = fused[ranking[rank].first];
            hit.id = ranking[rank].first;
            hit.score += 1.0f / (RRF_K + float(rank) + 1.0f);
            if (lexicalSide) hit.lexical = rank;
            else { hit.semantic = rank; hit.similarity = ranking[rank].second; }
        }
    };
    rankIn(keyword, true);
    rankIn(vectors, false);
    QVector<Hit> hits(fused.begin(), fused.end());
    std::sort(hits.begin(), hits.end(), [](const Hit &a, const Hit &b) { return a.score != b.score ? a.score > b.score : a.id < b.id; });
    if (hits.size() > limit) hits.resize(limit);
    for (Hit &hit : hits) hit.snippet = snippetFor(index, hit.id, query);
    return hits;
}

std::optional<std::pair<int, int>> highlightOf(const QString &snippet, const QString &query) {
    const QString lowered = snippet.toLower();
    const QString whole = query.trimmed().toLower();
    QStringList candidates;
    if (!whole.isEmpty()) candidates.append(whole);
    QStringList terms = tokenize(query);
    std::stable_sort(terms.begin(), terms.end(), [](const QString &a, const QString &b) { return a.size() > b.size(); });
    candidates.append(terms);
    for (const QString &candidate : candidates) {
        const int at = lowered.indexOf(candidate);
        if (at < 0) continue;
        return std::make_pair(at, at + int(candidate.size()));
    }
    return std::nullopt;
}

} // namespace brain::search
