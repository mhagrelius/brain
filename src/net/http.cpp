#include "http.h"

#include <QHostAddress>
#include <QHostInfo>
#include <QTcpSocket>
#include <QUrl>

namespace brain::net {

namespace {
bool readUntilHeaders(QTcpSocket &socket, QByteArray &buffer, int timeoutMs) {
    while (buffer.indexOf("\r\n\r\n") < 0) {
        if (socket.bytesAvailable() == 0 && !socket.waitForReadyRead(timeoutMs)) return false;
        buffer.append(socket.readAll());
        if (buffer.size() > 1 << 20) return false;
    }
    return true;
}
}

std::optional<Response> send(const Request &request, QString *error) {
    const QUrl url(request.url);
    if (!url.isValid() || url.host().isEmpty()) { if (error) *error = request.url + QStringLiteral(" is not a URL"); return std::nullopt; }
    if (url.scheme() != QLatin1String("http")) { if (error) *error = QStringLiteral("only http:// is spoken (") + request.url + u')'; return std::nullopt; }
    const int port = url.port(80);

    QTcpSocket socket;
    socket.connectToHost(url.host(), quint16(port));
    if (!socket.waitForConnected(request.timeoutMs)) { if (error) *error = QStringLiteral("no answer from ") + url.host() + u':' + QString::number(port) + QStringLiteral(": ") + socket.errorString(); return std::nullopt; }

    QString path = url.path(QUrl::FullyEncoded);
    if (path.isEmpty()) path = QStringLiteral("/");
    if (url.hasQuery()) path += u'?' + url.query(QUrl::FullyEncoded);
    QByteArray head;
    head += request.method.toLatin1() + ' ' + path.toLatin1() + " HTTP/1.1\r\n";
    head += "Host: " + url.host().toLatin1() + (port == 80 ? QByteArray() : ':' + QByteArray::number(port)) + "\r\n";
    head += "Connection: close\r\n";
    head += "Accept: application/json\r\n";
    if (!request.bearer.isEmpty()) head += "Authorization: Bearer " + request.bearer.toUtf8() + "\r\n";
    if (!request.body.isEmpty() || request.method != QLatin1String("GET")) {
        head += "Content-Type: application/json\r\n";
        head += "Content-Length: " + QByteArray::number(request.body.size()) + "\r\n";
    }
    head += "\r\n";
    socket.write(head);
    socket.write(request.body);
    if (!socket.waitForBytesWritten(request.timeoutMs)) { if (error) *error = QStringLiteral("could not send to ") + url.host(); return std::nullopt; }

    QByteArray buffer;
    if (!readUntilHeaders(socket, buffer, request.timeoutMs)) { if (error) *error = QStringLiteral("no reply from ") + url.host(); return std::nullopt; }
    const int headerEnd = buffer.indexOf("\r\n\r\n");
    const QByteArray headers = buffer.left(headerEnd);
    QByteArray rest = buffer.mid(headerEnd + 4);

    Response response;
    const int firstLineEnd = headers.indexOf("\r\n");
    const QByteArray statusLine = firstLineEnd < 0 ? headers : headers.left(firstLineEnd);
    const QList<QByteArray> parts = statusLine.split(' ');
    if (parts.size() < 2) { if (error) *error = QStringLiteral("a reply that is not HTTP"); return std::nullopt; }
    response.status = parts[1].toInt();

    qint64 contentLength = -1;
    bool chunked = false;
    for (const QByteArray &line : headers.mid(firstLineEnd + 2).split('\n')) {
        const int colon = line.indexOf(':');
        if (colon < 0) continue;
        const QByteArray name = line.left(colon).trimmed().toLower();
        const QByteArray value = line.mid(colon + 1).trimmed();
        if (name == "content-length") contentLength = value.toLongLong();
        else if (name == "transfer-encoding" && value.toLower().contains("chunked")) chunked = true;
    }

    auto fill = [&](qint64 wanted) {
        while (rest.size() < wanted) {
            if (socket.bytesAvailable() == 0 && !socket.waitForReadyRead(request.timeoutMs)) return false;
            rest.append(socket.readAll());
        }
        return true;
    };

    if (chunked) {
        QByteArray body;
        for (;;) {
            int lineEnd;
            while ((lineEnd = rest.indexOf("\r\n")) < 0) {
                if (socket.bytesAvailable() == 0 && !socket.waitForReadyRead(request.timeoutMs)) { if (error) *error = QStringLiteral("the reply stopped early"); return std::nullopt; }
                rest.append(socket.readAll());
            }
            bool ok = false;
            const qint64 size = rest.left(lineEnd).split(';').first().trimmed().toLongLong(&ok, 16);
            rest.remove(0, lineEnd + 2);
            if (!ok) { if (error) *error = QStringLiteral("a chunk size that is not hex"); return std::nullopt; }
            if (size == 0) break;
            if (!fill(size + 2)) { if (error) *error = QStringLiteral("the reply stopped early"); return std::nullopt; }
            body.append(rest.left(size));
            rest.remove(0, int(size) + 2);
        }
        response.body = body;
    } else if (contentLength >= 0) {
        if (!fill(contentLength)) { if (error) *error = QStringLiteral("the reply stopped early"); return std::nullopt; }
        response.body = rest.left(int(contentLength));
    } else {
        // No length and not chunked: read until the server closes.
        while (socket.state() == QAbstractSocket::ConnectedState) {
            if (!socket.waitForReadyRead(request.timeoutMs)) break;
            rest.append(socket.readAll());
        }
        rest.append(socket.readAll());
        response.body = rest;
    }
    return response;
}

} // namespace brain::net
