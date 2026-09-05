#pragma once

// A blocking HTTP/1.1 client for worker threads. QTcpSocket's waitFor*
// calls need no event loop, which is what makes this usable from a detached
// std::thread; brain-server and llama.cpp both speak plain HTTP with
// Content-Length (chunked is handled too, to be safe).

#include <QByteArray>
#include <QString>

#include <optional>

namespace brain::net {

struct Response {
    int status = 0;
    QByteArray body;
};

struct Request {
    QString method = QStringLiteral("GET");
    QString url;
    QByteArray body;
    QString bearer;
    int timeoutMs = 15000;
};

// nullopt when no answer came back at all; `error` says why.
std::optional<Response> send(const Request &request, QString *error);

} // namespace brain::net
