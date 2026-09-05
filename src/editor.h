#pragma once

// The editing model. One TextEdit shows the note as source, always styled,
// with the syntax characters removed from the *display* everywhere except in
// the construct holding the caret. The source is canonical; the display and
// a display↔source offset map are derived from it and the caret, and edits
// the TextEdit makes to the display are mapped back onto the source.

#include "scanner.h"
#include "vault.h"

#include <QDateTime>
#include <QHash>
#include <QObject>
#include <QPointer>
#include <QSize>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickTextDocument>
#include <QTextDocument>
#include <QVariantList>
#include <QVector>

class Palette;

class Editor : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("Created by App")
    Q_PROPERTY(QQuickTextDocument *document READ document WRITE setDocument NOTIFY documentChanged)
    Q_PROPERTY(QQuickItem *textEdit READ textEdit WRITE setTextEdit NOTIFY documentChanged)
    Q_PROPERTY(bool reading READ reading WRITE setReading NOTIFY readingChanged)
    Q_PROPERTY(QString source READ source NOTIFY sourceChanged)
    Q_PROPERTY(int wordCount READ wordCount NOTIFY sourceChanged)
    Q_PROPERTY(QVariantList decorations READ decorations NOTIFY displayChanged)
    Q_PROPERTY(QVariantList embeds READ embeds NOTIFY displayChanged)
    Q_PROPERTY(QString completionQuery READ completionQuery NOTIFY completionChanged)
    Q_PROPERTY(int completionAnchor READ completionAnchor NOTIFY completionChanged)
    Q_PROPERTY(bool completing READ completing NOTIFY completionChanged)
    Q_PROPERTY(qreal contentWidth READ contentWidth WRITE setContentWidth NOTIFY displayChanged)
    Q_PROPERTY(QString vaultRoot READ vaultRoot WRITE setVaultRoot NOTIFY displayChanged)

public:
    explicit Editor(Palette *palette, QObject *parent = nullptr);

    QQuickTextDocument *document() const { return m_quickDocument; }
    void setDocument(QQuickTextDocument *document);
    QQuickItem *textEdit() const { return m_textEdit; }
    void setTextEdit(QQuickItem *item);
    bool reading() const { return m_reading; }
    void setReading(bool reading);
    QString source() const { return m_source; }
    int wordCount() const;
    QVariantList decorations() const { return m_decorations; }
    QVariantList embeds() const { return m_embeds; }
    QString completionQuery() const { return m_completionQuery; }
    int completionAnchor() const { return m_completionAnchor; }
    bool completing() const { return m_completing; }
    qreal contentWidth() const { return m_contentWidth; }
    void setContentWidth(qreal width);
    QString vaultRoot() const { return m_vaultRoot; }
    void setVaultRoot(const QString &root);

    // Replace the note. `caret` is a source offset.
    Q_INVOKABLE void load(const QString &body, int caret = 0);
    Q_INVOKABLE void cursorMoved(int displayPosition);
    Q_INVOKABLE bool continueList(int displayPosition);       // Enter: repeat the bullet, the next number, a fresh box
    Q_INVOKABLE bool backspaceBullet(int displayPosition);    // Backspace on an empty item removes its marker
    Q_INVOKABLE void placeCaret(int displayPosition) { if (!m_syncing) m_caret = sourceOffset(displayPosition); }
    Q_INVOKABLE void applyFormat(const QString &label, int selectionStart, int selectionEnd);
    Q_INVOKABLE QString linkAt(int displayPosition) const;   // wikilink target, or ""
    Q_INVOKABLE QString urlAt(int displayPosition) const;
    Q_INVOKABLE QString tagAt(int displayPosition) const;
    Q_INVOKABLE bool toggleTaskAt(int displayPosition);
    Q_INVOKABLE void acceptCompletion(const QString &title);
    Q_INVOKABLE void dismissCompletion();
    Q_INVOKABLE void insertAtCaret(const QString &text, int displayPosition = -1);
    Q_INVOKABLE void insertEmbed(const QString &name);
    Q_INVOKABLE int sourceOffset(int displayPosition) const;
    Q_INVOKABLE int displayOffset(int sourceOffset) const;
    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();
    int caret() const { return m_caret; }

    void refreshStyles();   // theme or scale changed

signals:
    void documentChanged();
    void readingChanged();
    void sourceChanged();
    void displayChanged();
    void completionChanged();
    // The user changed the source (typing, formats, completions).
    void edited();
    void followRequested(const QString &target);

private:
    struct Hidden { int start; int end; };
    struct Snapshot { QString source; int caret; };
    void onContentsChange(int position, int removed, int added);
    void refreshHidden();
    void recompute();
    void syncDocument(bool force);
    void rebuild(bool force = false);
    void applyFormats();
    void syncCaret();
    void updateCompletion();
    void replaceSource(int start, int end, const QString &text, int caretAfter);
    void pushUndo();
    void applyRenumber();
    QString documentText() const;
    QString lineOf(int sourceOffset, int *lineStart, int *lineEnd) const;
    bool isImage(const QString &name) const;
    QVector<Hidden> hiddenRanges() const;

    Palette *m_palette;
    QPointer<QQuickTextDocument> m_quickDocument;
    QPointer<QTextDocument> m_document;
    QPointer<QQuickItem> m_textEdit;
    bool m_syncing = false;
    bool m_syncPending = false;
    bool m_reading = false;
    QVector<Snapshot> m_undo;
    QVector<Snapshot> m_redo;
    qint64 m_lastEditMs = 0;
    QString m_source;
    int m_caret = 0;
    brain::md::Parsed m_parsed;
    QString m_display;
    QVector<int> m_toSource;    // display index → source index; size display+1
    QVector<int> m_toDisplay;   // source index → display index; size source+1
    QVector<Hidden> m_hidden;   // the ranges the document currently reflects
    QVariantList m_decorations;
    QVariantList m_embeds;
    QString m_completionQuery;
    int m_completionAnchor = 0;
    bool m_completing = false;
    qreal m_contentWidth = 700;
    QString m_vaultRoot;
    QHash<QString, QSize> m_imageSizes;
};
