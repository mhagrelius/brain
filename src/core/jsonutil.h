#pragma once

// JSON carries u64 hashes and digests that a double cannot hold. The Rust
// server and client write them as bare integers; Qt's parser keeps integers
// only up to qint64. So before parsing, every bare integer of sixteen or more
// digits is quoted, and the reader takes it back with toULongLong().

#include <QByteArray>
#include <QJsonValue>

namespace brain::json {

QByteArray quoteBigIntegers(const QByteArray &raw);
quint64 toU64(const QJsonValue &value);

} // namespace brain::json
