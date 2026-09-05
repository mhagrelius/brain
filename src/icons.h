#pragma once

#include <QQuickImageProvider>

// Symbolic icons recoloured at render time: `image://icon/<name>/<#rrggbb>`
// renders src/icons/<name>.svg with its fill replaced, so a row never ships
// per-colour copies and re-tints when the theme changes. Needs Qt6::Svg.
class IconProvider : public QQuickImageProvider {
public:
    IconProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}
    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;
};
