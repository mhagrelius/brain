#include "jsonutil.h"

namespace brain::json {

QByteArray quoteBigIntegers(const QByteArray &raw) {
    QByteArray out;
    out.reserve(raw.size() + 64);
    const int n = raw.size();
    int i = 0;
    bool inString = false;
    while (i < n) {
        const char c = raw[i];
        if (inString) {
            out.append(c);
            if (c == '\\' && i + 1 < n) { out.append(raw[i + 1]); i += 2; continue; }
            if (c == '"') inString = false;
            ++i;
            continue;
        }
        if (c == '"') { inString = true; out.append(c); ++i; continue; }
        if (c >= '0' && c <= '9') {
            const char before = i > 0 ? raw[i - 1] : ' ';
            int j = i;
            while (j < n && raw[j] >= '0' && raw[j] <= '9') ++j;
            const char after = j < n ? raw[j] : ' ';
            const bool bare = !(before == '.' || (before >= '0' && before <= '9') || before == '-') && !(after == '.' || after == 'e' || after == 'E');
            if (bare && j - i >= 16) { out.append('"'); out.append(raw.mid(i, j - i)); out.append('"'); }
            else out.append(raw.mid(i, j - i));
            i = j;
            continue;
        }
        out.append(c);
        ++i;
    }
    return out;
}

quint64 toU64(const QJsonValue &value) {
    if (value.isString()) return value.toString().toULongLong();
    if (value.isDouble()) {
        const double d = value.toDouble();
        if (d >= 0 && d < 9.2e18) return quint64(value.toInteger());
        return quint64(d);
    }
    return 0;
}

} // namespace brain::json
